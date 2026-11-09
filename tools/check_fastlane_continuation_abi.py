#!/usr/bin/env python3
"""Check the emitted M1 yield ABI; a live syscall stack must never be repivoted."""
import re
import subprocess
import sys


def check(ops):
    ops = [re.sub(r"\s+", " ", op.strip()) for op in ops]
    assert len(ops) == 10, ops
    assert ops[:2] == ["push rbx", "mov rdx,rsp"], ops
    assert ops[2].startswith("call ") and "prepare_yield" in ops[2], ops
    assert ops[3:7] == ["mov rbx,rax", "int 0xf3", "mov rdi,rsp", "mov rsi,rbx"], ops
    assert ops[7].startswith("call ") and "resume_yield" in ops[7], ops
    assert ops[8:] == ["pop rbx", "ret"], ops


def main():
    elf = sys.argv[1]
    symbols = subprocess.check_output(["nm", elf], text=True)
    matches = [line.split()[-1] for line in symbols.splitlines()
               if "yield_on_kernel_stack" in line and line.split()[1].lower() == "t"]
    assert len(matches) == 1, matches
    assembly = subprocess.check_output(
        ["objdump", "-d", "-Mintel", "--no-show-raw-insn", "--disassemble=" + matches[0], elf],
        text=True,
    )
    ops = [m.group(1) for line in assembly.splitlines()
           if (m := re.match(r"\s*[0-9a-f]+:\s+(.+)", line))]
    print(assembly)
    check(ops)
    # Mutations must fail even when calls and the interrupt are still present.
    for broken in (ops[:3] + ["mov rsp,rax"] + ops[3:],
                   [op for op in ops if "resume_yield" not in op],
                   [re.sub(r"int\s+0xf3", "int 0x20", op) for op in ops]):
        try:
            check(broken)
        except AssertionError:
            continue
        raise AssertionError("unsafe continuation ABI control accepted")
    print("M1 emitted continuation ABI and negative controls: PASS")


if __name__ == "__main__":
    main()
