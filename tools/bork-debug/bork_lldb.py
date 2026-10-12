"""Load manually: command script import /absolute/path/to/bork_lldb.py."""
import os
import sys
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import lldb
from bork_debug import Decoder, DebugError, arena_text, export, graph_args


def reader(target):
    endian={lldb.eByteOrderLittle:'little',lldb.eByteOrderBig:'big'}.get(target.GetByteOrder(),'unknown')
    process=target.GetProcess()
    def read(address,length):
        error=lldb.SBError()
        data=process.ReadMemory(address,length,error)
        if error.Fail(): raise DebugError(error.GetCString())
        return data
    return Decoder(read,target.GetAddressByteSize(),endian)


def snapshot(target,decoder=None):
    value=target.FindFirstGlobalVariable('bork_debug_registry')
    if not value.IsValid(): value=target.FindFirstGlobalVariable('bork_runtime::arena::debug::bork_debug_registry')
    if value.IsValid():
        address=value.GetLoadAddress()
    else:
        symbols=target.FindSymbols('bork_debug_registry')
        if not symbols.GetSize(): raise DebugError('bork_debug_registry missing; build with -g')
        address=symbols.GetContextAtIndex(0).GetSymbol().GetStartAddress().GetLoadAddress(target)
    if address == lldb.LLDB_INVALID_ADDRESS: raise DebugError('registry has no load address')
    return (decoder or reader(target)).snapshot(address)


def decoded(value):
    target=value.GetTarget()
    decoder=reader(target)
    address=value.GetLoadAddress()
    if address == lldb.LLDB_INVALID_ADDRESS: raise DebugError('value has no load address')
    return decoder.ref(address,snapshot(target,decoder).arenas,
                       value.GetTypeName().startswith('bork::Object<'))


def ref_summary(value,internal_dict,options=None):
    try: return decoded(value).summary()
    except (DebugError,ValueError) as exc: return 'Bork debug unavailable: '+str(exc)


class RefChildren:
    def __init__(self,value,internal_dict):
        self.value=value.GetNonSyntheticValue()
        self.update()

    def update(self):
        self.items=[self.value.GetChildAtIndex(i) for i in range(self.value.GetNumChildren())]
        try:
            ref=decoded(self.value)
            if ref.status == 'alive' and ref.object:
                ptr=self.value.GetChildMemberWithName('object')
                pointee=ptr.GetType().GetPointeeType()
                if pointee.IsValid() and 0 < pointee.GetByteSize() <= ref.available:
                    payload=ptr.Dereference()
                    if payload.IsValid() and payload.GetError().Success(): self.items.append(payload)
        except (DebugError,ValueError):
            pass
        return False

    def num_children(self,max_children=None): return len(self.items)
    def has_children(self): return bool(self.items)
    def get_child_at_index(self,index): return self.items[index] if 0 <= index < len(self.items) else None
    def get_child_index(self,name):
        for index,item in enumerate(self.items):
            if item.GetName() == name: return index
        return -1


def arenas_command(debugger,command,exe_ctx,result,internal_dict):
    try:
        if command.strip(): raise DebugError('usage: bork-arenas')
        for arena in snapshot(debugger.GetSelectedTarget()).arenas.values(): result.AppendMessage(arena_text(arena))
    except (DebugError,ValueError) as exc: result.SetError(str(exc))


def graph_command(debugger,command,exe_ctx,result,internal_dict):
    try:
        dot,svg=graph_args(command)
        export(snapshot(debugger.GetSelectedTarget()),dot,svg)
        result.AppendMessage('Saved '+dot+(' and '+svg if svg else ''))
    except (DebugError,OSError,ValueError) as exc: result.SetError(str(exc))


def __lldb_init_module(debugger,internal_dict):
    debugger.HandleCommand('type category define bork')
    debugger.HandleCommand('type summary add -e -w bork -x "^bork::(Ref|Object)<.*>$" -F bork_lldb.ref_summary')
    debugger.HandleCommand('type synthetic add -w bork -x "^bork::(Ref|Object)<.*>$" -l bork_lldb.RefChildren')
    debugger.HandleCommand('type category enable bork')
    debugger.HandleCommand('command script add -f bork_lldb.arenas_command bork-arenas')
    debugger.HandleCommand('command script add -f bork_lldb.graph_command bork-graph')
