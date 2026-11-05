#!/usr/bin/env python3
"""Check the emitted entry instructions, not merely assembly source strings."""
import re
import subprocess
import sys


def check(instructions):
    ops = [re.sub(r'\s+', ' ', op.strip()) for op in instructions]
    assert ops[0] == 'swapgs', ops[:4]
    assert ops[1] == 'mov QWORD PTR gs:0x8,rsp', ops[1]
    pivot = ops.index('mov rsp,QWORD PTR gs:0x0')
    assert all(not op.startswith(('push', 'call')) for op in ops[:pivot])
    swaps = [i for i, op in enumerate(ops) if op == 'swapgs']
    assert len(swaps) == 2 and swaps[1] > pivot
    assert ops[pivot + 1:pivot + 4] == [
        'sub rsp,0x100', 'and rsp,0xfffffffffffffff0', 'push QWORD PTR gs:0x8']
    saved = [op for op in ops[swaps[1] + 1:] if op.startswith('push ')]
    assert saved == ['push ' + reg for reg in (
        'r15', 'r14', 'r13', 'r12', 'r11', 'r10', 'r9', 'r8',
        'rbp', 'rdi', 'rsi', 'rdx', 'rcx', 'rbx', 'rax')], saved
    call = next(i for i, op in enumerate(ops) if op.startswith('call '))
    assert call > swaps[1] and ops[call - 1] == 'and rsp,0xfffffffffffffff0'
    ret = next(i for i, op in enumerate(ops) if op.startswith('sysret'))
    assert ops[ret - 1] == 'pop rsp'
    assert ops[ret - 2] == 'or r11,0x200'


def main():
    elf = sys.argv[1]
    symbols = subprocess.check_output(['nm', elf], text=True)
    symbol = next((line.split()[-1] for line in symbols.splitlines()
                   if 'syscall_entry' in line and line.split()[1].lower() == 't'), None)
    command = ['objdump', '-d', '-Mintel', '--no-show-raw-insn']
    if symbol is not None:
        command.append('--disassemble=' + symbol)
    assembly = subprocess.check_output(command + [elf], text=True)
    ops = [m.group(1) for line in assembly.splitlines()
           if (m := re.match(r'\s*[0-9a-f]+:\s+(.+)', line))]
    ops = [re.sub(r'\s+', ' ', op.strip()) for op in ops]
    if symbol is None:
        # Ordinary release uses strip=true. Require exactly one emitted entry
        # signature, then inspect its entire entry/return sequence as usual.
        entries = [i for i in range(len(ops) - 1)
                   if ops[i:i + 2] == ['swapgs', 'mov QWORD PTR gs:0x8,rsp']]
        assert len(entries) == 1, entries
        begin = entries[0]
        end = next(i for i in range(begin, len(ops)) if ops[i].startswith('sysret'))
        ops = ops[begin:end + 1]
    print('\n'.join(ops))
    check(ops)
    # Reject the audited vulnerable sequence and representative regressions.
    for broken in (["push r15"] + ops, [op for op in ops if op != 'pop rsp'],
                   [op for op in ops if op != 'swapgs']):
        try:
            check(broken)
        except (AssertionError, ValueError, StopIteration):
            continue
        raise AssertionError('negative ABI control accepted')
    print('Emitted syscall ABI and negative controls: PASS')


if __name__ == '__main__':
    main()
