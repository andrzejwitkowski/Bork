import dataclasses
import pathlib
import tempfile
import shutil
import unittest
import unittest.mock
import bork_debug as d


class Memory:
    def __init__(self, size=8, endian='little'):
        self.size, self.endian = size, endian
        self.data = bytearray(16384)
        self.reads = []
    def words(self, at, *values):
        for i, value in enumerate(values):
            self.data[at+i*self.size:at+(i+1)*self.size] = value.to_bytes(self.size, self.endian)
    def read(self, at, size):
        self.reads.append((at, size))
        if at < 0 or at+size > len(self.data):
            raise RuntimeError('unmapped')
        return bytes(self.data[at:at+size])
    def fixture(self):
        self.words(64, 2, 2, 256, 2, 1024, 7)
        self.words(256, 1, 0, 0, 4, 3, 0, 0, 0, 6000, 1, 128, 512, 3000, 12)
        self.words(256+14*self.size, 2, 1, 1, 0, 1, 0, 2, 2, 0, 0, 0, 0, 0, 0)
        self.words(6000, 4096, 128, 512)
        text = b'a"b\\c.bk\0'
        self.data[3000:3000+len(text)] = text
        self.data[3100:3106] = b'value\0'
        for i, (control, kind) in enumerate([(0,0),(1,1),(1,2),(1,3),(1,4),(2,4),(99,4)]):
            address = 2048+i*32
            self.words(1024+i*6*self.size, 1, address, 3100, 0, 0, i)
            self.words(address, control, 4096)
            self.data[address+2*self.size] = kind
        return self


class DecoderTests(unittest.TestCase):
    def test_decodes_literal_target_layout_without_reading_payload(self):
        for size, endian in [(8,'little'),(4,'big')]:
            m = Memory(size,endian).fixture()
            snap = d.Decoder(m.read,size,endian).snapshot(64)
            self.assertEqual([s.ref.kind for s in snap.slots], ['None','Arena','Borrow','Strong','Weak','Weak','Weak'])
            self.assertEqual([s.ref.status for s in snap.slots], ['none','alive','alive','alive','alive','expired','stale'])
            self.assertEqual(snap.arenas[1].source, 'a"b\\c.bk')
            self.assertEqual(snap.slots[6].index, 6)
            self.assertFalse(any(a == 4096 for a,n in m.reads))
    def test_rejects_corrupt_and_inconsistent_snapshots(self):
        for words, message in [((3,2,256,2,1024,7),'ABI'),((2,3,256,2,1024,7),'mutation'),((2,2,256,1000001,1024,7),'count'),((2,2,999999,2,1024,7),'read')]:
            m = Memory().fixture(); m.words(64,*words)
            with self.assertRaisesRegex(d.DebugError,message):
                d.Decoder(m.read,8,'little').snapshot(64)
        for size,endian in [(16,'little'),(8,'middle')]:
            with self.assertRaises(d.DebugError): d.Decoder(Memory().read,size,endian)
    def test_detects_sequence_change_and_short_reads(self):
        m = Memory().fixture()
        def changing(at,size):
            result=m.read(at,size)
            if at == 1024: m.words(72,4)
            return result
        with self.assertRaisesRegex(d.DebugError,'changed'): d.Decoder(changing,8,'little').snapshot(64)
        with self.assertRaisesRegex(d.DebugError,'read'): d.Decoder(lambda a,n:b'',8,'little').snapshot(64)
    def test_rejects_live_object_outside_allocation_and_skips_dead_owner_slots(self):
        m=Memory().fixture()
        m.words(2048+32+8,8192)
        snap=d.Decoder(m.read,8,'little').snapshot(64)
        self.assertEqual(snap.slots[1].ref.status,'invalid object')
        self.assertEqual(snap.slots[2].ref.available,128)
        m.words(1024,2,999999,0,0,0,0)
        snap=d.Decoder(m.read,8,'little').snapshot(64)
        self.assertEqual(snap.slots[0].ref.status,'non-Alive owner')
        self.assertFalse(any(a == 999999 for a,n in m.reads))

    def test_graph_only_names_ref_targets_and_distinguishes_ref_kinds(self):
        m=Memory().fixture(); snap=d.Decoder(m.read,8,'little').snapshot(64)
        snap.arenas[3]=dataclasses.replace(snap.arenas[1],id=3,extents=((8192,8192,4096),))
        dot=d.to_dot(snap)
        self.assertNotIn('o3_8192',dot)
        self.assertIn('s1 -> o1_4096 [style=solid',dot)
        self.assertIn('s3 -> o1_4096 [style=bold',dot)
        self.assertIn('s4 -> o1_4096 [style=dashed',dot)
        self.assertIn('Weak dashed',dot)
        m.words(2048+32+8,0)
        snap=d.Decoder(m.read,8,'little').snapshot(64)
        self.assertEqual(snap.slots[1].ref.status,'invalid object')

    def test_zero_size_object_at_allocation_endpoint_remains_visible(self):
        m=Memory().fixture(); m.words(2048+32+8,4224)
        snap=d.Decoder(m.read,8,'little').snapshot(64)
        self.assertEqual(snap.slots[1].ref.status,'alive')
        self.assertEqual(snap.slots[1].ref.available,0)
        self.assertIn('o1_4224',d.to_dot(snap))

    def test_dot_preserves_utf8_source_names(self):
        m=Memory().fixture()
        source='żółć.bk'.encode()+b'\0'
        m.data[3000:3000+len(source)]=source
        snap=d.Decoder(m.read,8,'little').snapshot(64)
        self.assertIn('żółć.bk',d.to_dot(snap))

    def test_dot_preserves_slots_ownership_and_escapes(self):
        m = Memory().fixture(); snap=d.Decoder(m.read,8,'little').snapshot(64)
        dot=d.to_dot(snap)
        self.assertIn('style=dashed',dot)
        self.assertIn('style=bold',dot)
        self.assertIn('None / none',dot)
        self.assertIn('Weak / expired',dot)
        self.assertIn('value[6]',dot)
        self.assertIn('a\\"b\\\\c.bk:12',dot)
        self.assertIn('runtime strong=4 weak=3',dot)
        self.assertIn('observed Ref edges=',dot)
        self.assertIn('a1 -> o1_4096',dot)
        with tempfile.TemporaryDirectory() as tmp:
            path=pathlib.Path(tmp)/'graph.dot'
            d.export(snap,str(path))
            self.assertEqual(path.read_text(),dot)
            if shutil.which('dot'):
                svg=str(path)+'.svg'
                d.export(snap,str(path),svg)
                self.assertIn('<svg',pathlib.Path(svg).read_text())
            with unittest.mock.patch('bork_debug.subprocess.run',side_effect=FileNotFoundError):
                with self.assertRaisesRegex(d.DebugError,'DOT saved'): d.export(snap,str(path),str(path)+'.svg')
            self.assertEqual(path.read_text(),dot)


if __name__ == '__main__': unittest.main()
