"""Memory-only reader for the Bork debug registry ABI v1."""
from dataclasses import dataclass
import json
import pathlib
import shlex
import subprocess


class DebugError(Exception):
    pass


@dataclass
class Arena:
    id: int
    parent: int
    depth: int
    strong_count: int
    weak_count: int
    live_scoped_children: int
    state: int
    mode: int
    payload: int
    used: int
    capacity: int
    source: str
    line: int


@dataclass
class Ref:
    control: int
    object: int
    kind: str
    status: str
    available: int = 0

    def summary(self):
        return f'{self.kind} / {self.status}, arena={self.control}, object=0x{self.object:x}'


@dataclass
class Slot:
    owner: int
    address: int
    label: str
    type_name: str
    scope: int
    index: object
    ref: Ref


@dataclass
class Snapshot:
    arenas: dict
    slots: list


class Decoder:
    def __init__(self, read, pointer_size, endian):
        if pointer_size not in (4,8) or endian not in ('little','big'):
            raise DebugError('unsupported target pointer size or endian')
        self.reader, self.size, self.endian = read,pointer_size,endian
        self.maximum = (1 << (8*pointer_size))-1

    def read(self, address, length):
        if address <= 0 or length < 0 or address > self.maximum-length:
            raise DebugError('invalid memory read bounds')
        try:
            data=self.reader(address,length)
            if len(data) != length: raise ValueError('short read')
            return data
        except Exception as exc:
            raise DebugError(f'memory read at 0x{address:x}: {exc}') from exc

    def words(self, address, count):
        data=self.read(address,count*self.size)
        return [int.from_bytes(data[i*self.size:(i+1)*self.size],self.endian) for i in range(count)]

    def string(self,address):
        if not address: return ''
        data=bytearray()
        for i in range(4096):
            byte=self.read(address+i,1)
            if byte == b'\0': return data.decode('utf-8',errors='replace')
            data.extend(byte)
        raise DebugError('string exceeds 4096 bytes')

    def ref(self,address,arenas,object_descriptor=False):
        control,obj=self.words(address,2)
        kind=1 if object_descriptor else self.read(address+2*self.size,1)[0]
        if kind > 4: raise DebugError(f'unknown Ref kind {kind}')
        arena=arenas.get(control)
        status='none' if kind == 0 else 'stale' if arena is None else 'alive' if arena.state == 0 else 'expired' if kind == 4 else 'non-Alive'
        available=0
        if status == 'alive':
            if arena.payload <= obj <= arena.payload+arena.used and arena.payload:
                available=arena.payload+arena.used-obj
            else:
                status='invalid object'
        return Ref(control,obj,('None','Arena','Borrow','Strong','Weak')[kind],status,available)

    def snapshot(self,address):
        version,sequence,ap,ac,sp,sc=self.words(address,6)
        if version != 1: raise DebugError(f'unsupported debug ABI {version}')
        if sequence & 1: raise DebugError('registry mutation in progress; stop at a stable point')
        if ac > 1000000 or sc > 1000000: raise DebugError('registry count exceeds limit')
        arenas={}
        for i in range(ac):
            values=self.words(ap+i*13*self.size,13)
            if values[6] > 2 or values[7] > 3 or not values[0] or values[0] in arenas:
                raise DebugError('invalid arena record')
            if values[9] > values[10] or values[8] > self.maximum-values[10]:
                raise DebugError('invalid arena allocation bounds')
            values[11]=self.string(values[11])
            arena=Arena(*values); arenas[arena.id]=arena
        slots=[]
        for i in range(sc):
            owner,addr,label,typ,scope,index=self.words(sp+i*6*self.size,6)
            ref=self.ref(addr,arenas) if owner in arenas and arenas[owner].state == 0 else Ref(0,0,'Unknown','non-Alive owner')
            slots.append(Slot(owner,addr,self.string(label),self.string(typ),scope,None if index == self.maximum else index,ref))
        if self.words(address,6) != [version,sequence,ap,ac,sp,sc]:
            raise DebugError('registry changed while reading')
        return Snapshot(arenas,slots)


def arena_text(arena):
    return (f'arena {arena.id} parent={arena.parent} {("Alive","Destroying","Dead")[arena.state]} '
            f'mode={("root","lexical","dynamicfree","dynamicscoped")[arena.mode]} '
            f'runtime strong={arena.strong_count} weak={arena.weak_count} scoped={arena.live_scoped_children} '
            f'bytes={arena.used}/{arena.capacity} {arena.source}:{arena.line}')


def to_dot(snapshot):
    quote=lambda value: json.dumps(value,ensure_ascii=False)
    lines=['digraph bork {', '  graph [label="Arena ownership and persistent Ref slots; parent dashed gray; Weak dashed; Strong bold; Arena/Borrow solid; runtime counters differ from observed Ref edges"];']
    objects=set()
    for arena in snapshot.arenas.values():
        count=sum(s.ref.control == arena.id and s.ref.kind != 'None' for s in snapshot.slots)
        lines.append(f'  a{arena.id} [shape=box,label={quote(arena_text(arena)+chr(10)+f"observed Ref edges={count}")}];')
        if arena.parent:
            lines.append(f'  a{arena.parent} -> a{arena.id} [style=dashed,color=gray,label="parent"];')
    for i,slot in enumerate(snapshot.slots):
        label=slot.label or f'0x{slot.address:x}'
        if slot.index is not None: label+=f'[{slot.index}]'
        lines.append(f'  s{i} [shape=note,label={quote(label+" "+slot.type_name+chr(10)+slot.ref.summary())}];')
        lines.append(f'  a{slot.owner} -> s{i} [style=dotted,label="slot"];')
        ref=slot.ref
        style={"Weak":"dashed","Strong":"bold"}.get(ref.kind,"solid")
        if ref.status == 'alive' and ref.object:
            objects.add((ref.control,ref.object))
            lines.append(f'  s{i} -> o{ref.control}_{ref.object} [style={style},label={quote(ref.kind)}];')
        elif ref.kind != 'None' and ref.control in snapshot.arenas:
            lines.append(f'  s{i} -> a{ref.control} [style={style},label={quote(ref.kind+" / "+ref.status)}];')
    for owner,address in sorted(objects):
        lines.append(f'  o{owner}_{address} [label="object 0x{address:x}"];')
        lines.append(f'  a{owner} -> o{owner}_{address} [label="owns"];')
    return '\n'.join(lines+['}', ''])


def export(snapshot,dot_path,svg_path=None):
    pathlib.Path(dot_path).write_text(to_dot(snapshot),encoding='utf-8')
    if svg_path:
        try:
            result=subprocess.run(['dot','-Tsvg',dot_path,'-o',svg_path],capture_output=True,text=True,check=False)
            if result.returncode: raise DebugError(result.stderr.strip() or 'dot failed')
        except (OSError,DebugError) as exc:
            raise DebugError(f'DOT saved to {dot_path}; SVG unavailable: {exc}') from exc


def graph_args(command):
    args=shlex.split(command)
    if len(args) == 1: return args[0],None
    if len(args) == 3 and args[1] == '--svg': return args[0],args[2]
    raise DebugError('usage: bork-graph <file.dot> [--svg file.svg]')
