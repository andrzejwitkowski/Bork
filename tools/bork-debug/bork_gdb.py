"""Load manually: source /absolute/path/to/bork_gdb.py."""
import os
import sys
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import gdb
import gdb.printing
from bork_debug import Decoder, DebugError, arena_text, export, graph_args


def reader():
    endian=gdb.execute('show endian',to_string=True)
    byteorder='little' if 'little endian' in endian else 'big' if 'big endian' in endian else 'unknown'
    return Decoder(lambda address,length: bytes(gdb.selected_inferior().read_memory(address,length)),
                   gdb.lookup_type('void').pointer().sizeof,byteorder)


def snapshot(decoder=None):
    symbol=(gdb.lookup_global_symbol('bork_debug_registry') or
            gdb.lookup_global_symbol('bork_runtime::arena::debug::bork_debug_registry'))
    if symbol is not None:
        address=int(symbol.value().address)
    else:
        try: address=int(gdb.parse_and_eval("&'bork_debug_registry'"))
        except gdb.error as exc: raise DebugError('bork_debug_registry missing; build with -g') from exc
    return (decoder or reader()).snapshot(address)


def type_name(value):
    typ=value.type.strip_typedefs()
    return typ.tag or typ.name or str(typ)


class RefPrinter:
    def __init__(self,value):
        self.value=value
        self.ref=None
        self.error=None
        try:
            decoder=reader()
            self.ref=decoder.ref(int(value.address),snapshot(decoder).arenas,
                                 type_name(value).startswith('bork::Object<'))
        except (DebugError,gdb.error,ValueError) as exc:
            self.error=str(exc)

    def to_string(self):
        return self.ref.summary() if self.ref else 'Bork debug unavailable: '+self.error

    def children(self):
        names=('arena','object') if type_name(self.value).startswith('bork::Object<') else ('control','object','kind')
        for name in names:
            yield name,gdb.Value(int(self.value[name])) if name in ('arena','control') else self.value[name]
        if self.ref and self.ref.status == 'alive' and self.ref.object:
            ptr=self.value['object']
            try:
                if ptr.type.target().code != gdb.TYPE_CODE_VOID and 0 < ptr.type.target().sizeof <= self.ref.available:
                    yield 'payload',ptr.dereference()
            except gdb.error:
                return


class ArenasCommand(gdb.Command):
    def __init__(self): super().__init__('bork-arenas',gdb.COMMAND_DATA)
    def invoke(self,args,from_tty):
        try:
            if args.strip(): raise DebugError('usage: bork-arenas')
            for arena in snapshot().arenas.values(): gdb.write(arena_text(arena)+'\n')
        except (DebugError,gdb.error) as exc: raise gdb.GdbError(str(exc))


class GraphCommand(gdb.Command):
    def __init__(self): super().__init__('bork-graph',gdb.COMMAND_DATA)
    def invoke(self,args,from_tty):
        try:
            dot,svg=graph_args(args)
            export(snapshot(),dot,svg)
            gdb.write('Saved '+dot+(' and '+svg if svg else '')+'\n')
        except (DebugError,gdb.error,OSError,ValueError) as exc: raise gdb.GdbError(str(exc))


printers=gdb.printing.RegexpCollectionPrettyPrinter('bork')
printers.add_printer('Ref',r'^bork::Ref<.*>$',RefPrinter)
printers.add_printer('Object',r'^bork::Object<.*>$',RefPrinter)
gdb.printing.register_pretty_printer(None,printers,replace=True)
ArenasCommand()
GraphCommand()
