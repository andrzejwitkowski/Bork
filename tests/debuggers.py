"""End-to-end checks of Bork-generated DWARF and runtime snapshots.

Run after cargo build --features codegen: python3 tests/debuggers.py
Set BORK_LLDB if LLDB is outside PATH. Both debuggers need Python support.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PROGRAM = '''class Node {
    value: i32
    next: Ref<Node>
    fun read(): i32 {
        return value
    }
}
fun checkpoint(node: &Node): i32 {
    return node.read()
}
fun main(): i32 {
    val first = Node(7, None)
    val second = Node(8, first)
    val reference: Ref<Node> = second
    val refs = [reference, None]
    return checkpoint(&second)
}
'''


class DebuggerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.source = self.directory / 'main.bork'
        self.binary = self.directory / 'main'
        self.compile_program(PROGRAM)

    def compile_program(self, program, exit_code=8):
        self.source.write_text(program)
        compiler = os.environ.get('BORK_TEST_COMPILER', str(ROOT / 'target/debug/bork'))
        built = subprocess.run([compiler, 'build', '-g', '-o', str(self.binary), str(self.source)], capture_output=True, text=True, timeout=60)
        self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
        self.assertEqual(subprocess.run([self.binary], timeout=30).returncode, exit_code)

    def run_debugger(self, kind):
        executable = shutil.which('gdb') if kind == 'gdb' else os.environ.get('BORK_LLDB', shutil.which('lldb'))
        if not executable:
            self.skipTest(kind + ' is unavailable')
        adapter = ROOT / 'tools/bork-debug' / ('bork_' + kind + '.py')
        before = self.directory / 'before.dot'
        after = self.directory / 'after.dot'
        svg = self.directory / 'graph.svg'
        common = ['bork-arenas', 'bork-graph ' + str(before), 'bork-arenas', 'bork-graph ' + str(after) + ' --svg ' + str(svg)]
        if kind == 'gdb':
            commands = ['set pagination off', 'set disable-randomization off', 'source ' + str(adapter), 'break ' + str(self.source) + ':9', 'run', 'print node'] + common + ['break Node.read', 'continue', 'print this', 'backtrace', 'continue']
            args = [executable, '-q', '-nx', '-batch', str(self.binary)]
            for command in commands:
                args += ['-ex', command]
        else:
            commands = ['settings set symbols.enable-external-lookup false', 'settings set target.disable-aslr false', 'target create ' + str(self.binary), 'command script import ' + str(adapter), 'breakpoint set --file main.bork --line 9', 'run', 'frame variable node'] + common + ['breakpoint set --basename Node.read', 'continue', 'frame variable this', 'bt', 'continue']
            args = [executable, '--no-lldbinit', '--batch']
            for command in commands:
                args += ['-o', command]
        environment = dict(os.environ, DEBUGINFOD_URLS='')
        result = subprocess.run(args, capture_output=True, text=True, timeout=90, env=environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('value = 8', result.stdout)
        self.assertIn('this', result.stdout)
        self.assertIn('Node.read', result.stdout)
        self.assertNotIn('Bork debug unavailable', result.stdout + result.stderr)
        self.assertNotIn('Cannot access memory', result.stdout + result.stderr)
        self.assertEqual(before.read_text(), after.read_text(), 'displaying values changed the graph')
        graph = before.read_text()
        self.assertIn('reference', graph)
        self.assertIn('Node.next', graph)
        self.assertIn('[0]', graph)
        self.assertIn('<svg', svg.read_text())
        return graph

    def test_source_stepping_restores_shadowed_variable(self):
        self.compile_program("""fun main(): i32 {
    val value = 1
    {
        val value = 2
        println(value)
    }
    println(value)
    return 0
}
""", exit_code=0)
        for kind in ('gdb', 'lldb'):
            executable = shutil.which('gdb') if kind == 'gdb' else os.environ.get('BORK_LLDB', shutil.which('lldb'))
            if not executable:
                self.skipTest(kind + ' is unavailable')
            if kind == 'gdb':
                commands = ['set disable-randomization off', 'break ' + str(self.source) + ':5', 'run', 'print value', 'next', 'frame', 'print value', 'continue']
                args = [executable, '-q', '-nx', '-batch', str(self.binary)]
                option = '-ex'
                values = ('$1 = 2', '$2 = 1')
            else:
                commands = ['settings set symbols.enable-external-lookup false', 'settings set target.disable-aslr false', 'target create ' + str(self.binary), 'breakpoint set --file main.bork --line 5', 'run', 'frame variable value', 'thread step-over', 'frame info', 'frame variable value', 'continue']
                args = [executable, '--no-lldbinit', '--batch']
                option = '-o'
                values = ('value = 2', 'value = 1')
            for command in commands:
                args += [option, command]
            result = subprocess.run(args, capture_output=True, text=True, timeout=90, env=dict(os.environ, DEBUGINFOD_URLS=''))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            for value in values:
                self.assertIn(value, result.stdout)
            self.assertIn('main.bork:7', result.stdout)

    def test_borrowed_array_call_preserves_registered_storage(self):
        program = PROGRAM.replace('fun main(): i32 {', 'fun touch(refs: &[Ref<Node>; 2]) { }\nfun main(): i32 {')
        program = program.replace('    return checkpoint(&second)', '    touch(&refs)\n    return checkpoint(&second)')
        self.compile_program(program)
        for debugger in ('gdb', 'lldb'):
            graph = self.run_debugger(debugger)
            self.assertIn('refs[1]', graph)

    def test_expired_weak_and_scope_cleanup_in_both_debuggers(self):
        expired = PROGRAM[:PROGRAM.index('fun main()')] + """fun main(): i32 {
    var reference: Ref<Node> = None
    {
        val parent = Node(7, None)
        val child = Node(8, parent)
        val old_local: Ref<Node> = parent
        reference = child
    }
    if val second = reference {
        val refs = [reference, None]
        return checkpoint(&second)
    }
    return 0
}
"""
        self.compile_program(expired)
        for debugger in ('gdb', 'lldb'):
            graph = self.run_debugger(debugger)
            self.assertIn('Weak / expired', graph)
            self.assertNotIn('old_local', graph)

    def test_both_debuggers_export_equivalent_snapshot(self):
        import re
        if not shutil.which('gdb') or not (os.environ.get('BORK_LLDB') or shutil.which('lldb')):
            self.skipTest('requires both debuggers')
        def normalize(graph):
            graph = re.sub(r'0x[0-9a-f]+', 'ADDRESS', graph)
            return re.sub(r'(o[0-9]+_)[0-9]+', r'\1ADDRESS', graph)
        self.assertEqual(normalize(self.run_debugger('gdb')), normalize(self.run_debugger('lldb')))

    def test_gdb_source_methods_refs_and_stable_snapshot(self):
        self.run_debugger('gdb')

    def test_lldb_source_methods_refs_and_stable_snapshot(self):
        self.run_debugger('lldb')


if __name__ == '__main__':
    unittest.main()
