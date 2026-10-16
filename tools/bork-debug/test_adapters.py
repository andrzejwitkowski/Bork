"""Runs the GDB adapter against a native process, rather than emulating GDB."""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest


CPP_FIXTURE = '''#include <cstdint>
namespace bork { template<class T> struct Ref { uintptr_t control; T* object; unsigned char kind; }; template<class T> struct Object { uintptr_t arena; T* object; }; }
struct Payload { int answer; };
struct Registry { uintptr_t version,sequence,arenas,arena_count,slots,slot_count; };
uintptr_t pages[]={0,0,64};
uintptr_t arenas[]={1,0,0,2,1,0,0,0,(uintptr_t)pages,1,0,64,0,0, 2,1,1,0,1,0,2,0,0,0,0,0,0,0};
namespace bork_runtime { namespace arena { namespace debug { extern "C" { Registry bork_debug_registry={2,0,(uintptr_t)arenas,2,0,0}; } } } }
Payload payload={42};
bork::Ref<Payload> live={1,&payload,3};
bork::Ref<Payload> expired={2,(Payload*)0x123,4};
bork::Object<Payload> object={1,&payload};
void stopped() {} int main() { pages[0]=(uintptr_t)&payload; pages[1]=sizeof(payload); arenas[10]=sizeof(payload); stopped(); }
'''

@unittest.skipUnless(shutil.which('gdb') and shutil.which('cc'), 'requires GDB and C compiler')
class GDBTests(unittest.TestCase):
    def test_commands_read_registry_and_export_graph(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp)
            source=root/'fixture.c'
            source.write_text('''#include <stdint.h>
struct Registry { uintptr_t version,sequence,arenas,arena_count,slots,slot_count; };
uintptr_t arena[]={1,0,0,2,0,0,0,0,0,0,0,64,0,0};
struct Registry bork_debug_registry={2,0,(uintptr_t)arena,1,0,0};
void stopped(void) {} int main(void) { stopped(); return 0; }
''')
            subprocess.run(['cc','-g','-O0',str(source),'-o',str(root/'fixture')],check=True)
            adapter=pathlib.Path(__file__).with_name('bork_gdb.py').resolve()
            result=subprocess.run(['gdb','-q','-batch',str(root/'fixture'),'-ex',f'source {adapter}', '-ex','break stopped','-ex','run','-ex','bork-arenas','-ex',f'bork-graph {root}/graph.dot'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertIn('runtime strong=2 weak=0',result.stdout)
            self.assertIn('a1 [shape=box', (root/'graph.dot').read_text())


@unittest.skipUnless(shutil.which('c++'), 'requires C++ compiler')
class PrinterTests(unittest.TestCase):
    def test_gdb_printers_expand_live_payload_and_never_expired_payload(self):
        if not shutil.which('gdb'): self.skipTest('requires GDB')
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp)
            source=root/'fixture.cpp'
            source.write_text(CPP_FIXTURE)
            subprocess.run(['c++','-g','-O0',str(source),'-o',str(root/'fixture')],check=True)
            adapter=pathlib.Path(__file__).with_name('bork_gdb.py').resolve()
            result=subprocess.run(['gdb','-q','-batch',str(root/'fixture'),'-ex',f'source {adapter}', '-ex','break stopped','-ex','run','-ex','print live','-ex','print expired','-ex','print object'],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertIn('Strong / alive',result.stdout)
            self.assertIn('answer = 42',result.stdout)
            self.assertIn('Weak / expired',result.stdout)
            self.assertIn('Arena / alive',result.stdout)
            self.assertNotIn('Cannot access memory',result.stdout+result.stderr)


@unittest.skipUnless(os.environ.get('BORK_LLDB') or shutil.which('lldb'), 'requires LLDB (or BORK_LLDB)')
class LLDBTests(unittest.TestCase):
    def test_commands_and_printers_read_native_memory(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp)
            source=root/'fixture.cpp'
            source.write_text(CPP_FIXTURE)
            subprocess.run(['c++','-g','-O0',str(source),'-o',str(root/'fixture')],check=True)
            adapter=pathlib.Path(__file__).with_name('bork_lldb.py').resolve()
            commands=['settings set target.disable-aslr false',f'command script import {adapter}',
                      'breakpoint set -n stopped','run','frame variable live expired object',
                      'bork-arenas',f'bork-graph {root}/graph.dot']
            args=[os.environ.get('BORK_LLDB') or shutil.which('lldb'),'-b',str(root/'fixture')]
            for command in commands: args.extend(['-o',command])
            result=subprocess.run(args,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertIn('Strong / alive',result.stdout)
            self.assertIn('answer = 42',result.stdout)
            self.assertIn('Weak / expired',result.stdout)
            self.assertIn('Arena / alive',result.stdout)
            self.assertIn('runtime strong=2 weak=1',result.stdout)
            self.assertIn('a1 [shape=box',(root/'graph.dot').read_text())
            self.assertNotIn('memory read failed',result.stdout+result.stderr)


if __name__ == '__main__': unittest.main()
