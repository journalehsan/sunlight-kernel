import array
import mmap
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile


root = Path(__file__).resolve().parents[2]
output = root / "target" / "fd-shm-pool"
output.mkdir(parents=True, exist_ok=True)
flags = [
    "-Wall", "-Wextra", "-Werror", "-Os", "-static", "-nostdlib",
    "-fno-pie", "-no-pie", "-ffreestanding", "-fno-stack-protector",
]

for backend, define in (("memfd", "HOST_POOL_PROBE"), ("temp", "HOST_TEMP_POOL")):
    binary = output / f"host-{backend}.elf"
    subprocess.run(
        [os.environ.get("CC", "cc"), *flags, "-DPOOL_FIXTURE", f"-D{define}",
         str(root / "tools/tests/fd-shared-mmap.c"), "-o", str(binary)],
        check=True,
    )
    with tempfile.TemporaryDirectory() as directory, socket.socket(socket.AF_UNIX) as listener:
        listener.bind(str(Path(directory) / "wl-shm-measurement.sock"))
        listener.listen()
        listener.settimeout(10)
        commands = Path(directory) / "syscalls.gdb"
        commands.write_text(
            "set startup-with-shell off\n"
            "catch syscall\ncommands\nsilent\n"
            'printf "syscall %ld rax=%ld args=%#lx,%#lx,%#lx,%#lx,%#lx,%#lx\\n", '
            "$orig_rax, $rax, $rdi, $rsi, $rdx, $r10, $r8, $r9\n"
            "continue\nend\nrun\n"
        )
        with (output / f"host-{backend}.log").open("w") as log:
            process = subprocess.Popen(
                ["gdb", "-nx", "-batch", "-x", str(commands), str(binary)],
                cwd=directory, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                peer, _ = listener.accept()
                with peer:
                    peer.settimeout(10)
                    request, control, flags_received, _ = peer.recvmsg(16, socket.CMSG_SPACE(4))
                    while len(request) < 16:
                        chunk = peer.recv(16 - len(request))
                        assert chunk
                        request += chunk
                    assert not flags_received
                    assert struct.unpack("<IIII", request) == (7, 16 << 16, 8, 8192)
                    [(level, kind, data)] = control
                    assert (level, kind) == (socket.SOL_SOCKET, socket.SCM_RIGHTS)
                    [fd] = array.array("i", data)
                    assert peer.recv(1) == b"A"
                    with mmap.mmap(fd, 8192) as view:
                        os.close(fd)
                        assert view[:] == bytes((31 + index * 19 + (index >> 12)) & 255
                                                for index in range(8192))
                        view[:] = bytes((97 + index * 19 + (index >> 12)) & 255
                                        for index in range(8192))
                        peer.sendall(b"B")
                assert process.wait(timeout=10) == 0
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
    trace = (output / f"host-{backend}.log").read_text()
    assert "exited normally" in trace
    observed = [int(line.split()[1]) for line in trace.splitlines()
                if line.startswith("syscall ") and "rax=-38 " in line]
    creation = [319] if backend == "memfd" else [257, 263]
    assert observed == [41, 42, *creation, 77, 9, 46, 3, 1, 0, 11, 3, 60], observed
    print(f"{backend}: {observed}")
