#!/usr/bin/env python3
"""Run arbitrary cc.py native ELFs in a fresh headless SunlightOS guest.

No disk, network, shared folder or existing VM is attached. --suite builds and
classifies all 21 historical executable cells plus focused runtime fixtures.
Single-ELF mode returns the observed guest status unless --expect-exit is set.
Infrastructure, timeout, exception and cleanup failures always return nonzero.
"""
import argparse
import csv
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import struct
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
WORK = '/tmp/native-c-work'
STDOUT = (b'stdio: native -42 42 0x2a -1234567890 7 %\n'
          b'stdio: puts\n!\nstdio: vprintf V\nstdio: file PASS; tmpfile PASS\n')


def rust_bytes(value):
    return '&[' + ','.join(map(str, value)) + ']'


def summarize_pcap(path):
    """Count link-layer/IP protocols in the QEMU filter-dump capture.

    Loopback TCP is expected to appear as zero TCP frames because the guest
    stack never transmits it to the virtual NIC.
    """
    data = path.read_bytes()
    if len(data) < 24 or data[:4] != b'\xd4\xc3\xb2\xa1':
        return 'unreadable pcap\n'
    offset = 24
    counts = {}
    while offset + 16 <= len(data):
        _, _, incl, _ = struct.unpack('<IIII', data[offset:offset + 16])
        offset += 16
        frame = data[offset:offset + incl]
        offset += incl
        if len(frame) < 14:
            continue
        ethertype = int.from_bytes(frame[12:14], 'big')
        if ethertype == 0x0800 and len(frame) >= 24:
            key = 'ipv4-proto-' + str(frame[23])
        else:
            key = 'ethertype-0x%04x' % ethertype
        counts[key] = counts.get(key, 0) + 1
    lines = ['frames=%d' % sum(counts.values())]
    lines += ['%s=%d' % (key, counts[key]) for key in sorted(counts)]
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    choice = parser.add_mutually_exclusive_group(required=True)
    choice.add_argument('elf', nargs='?', type=Path)
    choice.add_argument('--suite', action='store_true')
    parser.add_argument('--out', type=Path, help='new evidence directory; default: unique path under target/native-c-guest-runs')
    parser.add_argument('--cpus', type=int, default=2, choices=(1, 2, 4))
    parser.add_argument('--timeout', type=int, default=90, help='guest wall-clock seconds')
    parser.add_argument('--expect-exit', type=int)
    parser.add_argument('--arg', action='append', default=[])
    parser.add_argument('--stdin-file', type=Path)
    parser.add_argument('--locale-conf', type=Path, help='fixture contents for /etc/locale.conf')
    parser.add_argument('--env', action='append', default=[], help='single-ELF startup KEY=VALUE')
    parser.add_argument('--net', action='store_true',
                        help='attach virtio-net and run net_server plus --service as support processes')
    parser.add_argument('--net-subnet', metavar='A.B.C.D/PREFIX',
                        help='explicit QEMU user-mode IPv4 subnet for --net, '
                             'for example 10.0.3.0/24 (default: QEMU default 10.0.2.0/24)')
    parser.add_argument('--service', type=Path,
                        help='optional native C service ELF run as support PID 4 (requires --net)')
    opts = parser.parse_args()
    if opts.service and not opts.net:
        parser.error('--service requires --net')
    if opts.net_subnet and not opts.net:
        parser.error('--net-subnet requires --net')
    if opts.net_subnet:
        match = re.fullmatch(r'(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})/(\d{1,2})',
                             opts.net_subnet)
        if match is None:
            parser.error('--net-subnet must be A.B.C.D/PREFIX')
        octets = [int(part) for part in match.groups()[:4]]
        prefix = int(match.group(5))
        if any(octet > 255 for octet in octets) or not 8 <= prefix <= 30:
            parser.error('--net-subnet has an invalid address or prefix')
        network = int.from_bytes(bytes(octets), 'big')
        host_bits = 32 - prefix
        if network & ((1 << host_bits) - 1):
            parser.error('--net-subnet must be a network address with zero host bits')
    if opts.net and opts.suite:
        parser.error('--net is a single-ELF mode')
    out = (opts.out or ROOT / 'target/native-c-guest-runs' / uuid.uuid4().hex).resolve()
    out.mkdir(parents=True, exist_ok=False)
    (out / '.gitignore').write_text('*\n!*.log\n!*.csv\n!*.json\n!*.txt\n!.gitignore\n')
    print(f'Evidence: {out}', flush=True)
    (out / 'invocation.log').write_text('$ ' + shlex.join(sys.argv) + '\n')

    def command(name, argv, *, env=None, timeout=900):
        argv = list(map(str, argv))
        with (out / (name + '.log')).open('w') as stream:
            stream.write('$ ' + shlex.join(argv) + '\n')
            stream.flush()
            try:
                p = subprocess.run(argv, cwd=ROOT, env=env, stdout=stream,
                                   stderr=subprocess.STDOUT, timeout=timeout)
                code = p.returncode
            except (OSError, subprocess.TimeoutExpired) as error:
                stream.write(str(error) + '\n')
                code = 125
            stream.write('\nexit=' + str(code) + '\n')
        return code

    command('revision', ['git', 'rev-parse', 'HEAD'])
    command('worktree', ['git', 'status', '--short'])
    command('qemu-version', ['qemu-system-x86_64', '--version'])
    cases, rows = [], []

    def add(name, elf, expected=0, args=(), input=b'', stdout=None, stderr=None, historical=False, environment=(b'TMPDIR=/tmp',)):
        elf = elf.resolve()
        data = elf.read_bytes()
        # Keep personality rejection explicit rather than accidentally testing Helios.
        if (data[:7] != b'\x7fELF\x02\x01\x01' or data[7] != 0 or
                data[9:15] == b'HLNX01' or int.from_bytes(data[16:18], 'little') != 2 or
                int.from_bytes(data[18:20], 'little') != 62):
            raise ValueError(f'{elf}: expected a native x86-64 ET_EXEC from cc.py')
        cases.append(dict(name=name, elf=elf, expected=expected, args=list(args), input=input,
                          stdout=stdout, stderr=stderr, environment=environment))
        rows.append(dict(probe=name, historical=historical, build='PASS', guest_run='NOT_RUN',
                         acceptance='FAIL', exit='', expected_exit=expected, detail='',
                         sha256=hashlib.sha256(data).hexdigest()))

    if opts.suite:
        if command('build-runtime', ['bash', 'tools/build_native_c.sh']):
            return 1
        if command('build-cxx-output-runtime', ['bash', 'tools/build_native_cxx.sh']):
            return 1
        base = ROOT / 'docs/audit/probes'
        sources = [(name, base / 'native-foundation' / (name + '.c'), code, (), b'', stdout, stderr, True)
                   for name, code, stdout, stderr in (
                       ('entry', 0, b'', b''), ('write', 0, b'', b'[NATIVE-C] write OK\n'),
                       ('exit', 23, b'', b''))]
        sources += [(p.stem, p, 0, (), b'', b'atexit OK\n' if p.stem == 'atexit' else b'', b'', True)
                    for p in sorted(base.glob('*.c')) if p.name != 'compiler.c']
        sources += [(p.stem, p, 0, (), b'', b'', b'', True) for p in sorted(base.glob('*.cpp'))]
        # Independent service counters bracket the unchanged historical source.
        # No other workload runs between these snapshots and getrandom.c.
        random_fixture = ROOT / 'tools/native-c/tests/random.c'
        random_index = next(i for i, source in enumerate(sources) if source[0] == 'getrandom')
        sources.insert(random_index, ('random-service-before', random_fixture, 0, ('stats',), b'', None, b'', False))
        sources.insert(random_index + 2, ('random-service-after', random_fixture, 0, ('stats',), b'', None, b'', False))
        sources += [('termios-terminal-identity', ROOT / 'tools/native-c/tests/termios.c', 0, ('tty',), b'', b'', b'', False)]
        sources += [('cxx-output', ROOT / 'tools/native-c/tests/cxx-output.cpp', 0, (), b'',
                     b'output 0 1 -9223372036854775808 9223372036854775807 18446744073709551615 18446744073709551615\nbounds -32768 32767 65535 -2147483648 2147483647 4294967295 -9223372036854775808 9223372036854775807 0\nABC\n',
                     b'stderr\nclog\n', False),
                    ('cxx-backend', ROOT / 'tools/native-c/tests/cxx-backend.cpp', 0, (), b'',
                     b'partial\nabxy', b'', False)]
        sources += [('cxx-exception-class', ROOT / 'tools/native-c/tests/cxx-exception-class.cpp', 0, (), b'', b'', b'', False)]
        sources += [('cpp-runtime-native', base / 'cpp-runtime.cpp', 0, (), b'',
                     b'exception OK\nTLS OK\nstatic initialization OK\natexit OK\n', b'', False)]
        sources += [('cxx-runtime-execution', ROOT / 'tools/native-c/tests/cxx-runtime-execution.cpp', 0, (), b'', b'', b'', False)]
        sources += [('cxx-runtime-' + mode, ROOT / 'tools/native-c/tests/cxx-runtime-execution.cpp', 134, (mode,), b'', b'armed\n', b'', False)
                    for mode in ('uncaught', 'destructor', 'worker', 'storage')]
        sources += [('cxx-local-static-' + mode, ROOT / 'tools/native-c/tests/cxx-local-static.cpp', code,
                     (mode,), b'', stdout, None if mode == 'recursive' else b'', False)
                    for mode, code, stdout in (
                        ('basic', 0, b'basic:OK\nbasic:D\n'),
                        ('concurrent', 0, b'concurrent:OK\nconcurrent:D\n'),
                        ('retry', 0, b'retry:OK\nretry:D\n'),
                        ('nested', 0, b'nested:OK\nouter:D\ninner:D\n'),
                        ('order', 0, b'order:OK\nsecond:D\nfirst:D\n'),
                        ('recursive', 134, b'recursive:armed\n'),
                    )]
        sources += [('cxx-string', ROOT / 'tools/native-c/tests/cxx-string.cpp', 0, (), b'', b'string:A\0B:42\n', b'', False),
                    ('cxx-string-backend', ROOT / 'tools/native-c/tests/cxx-string-backend.cpp', 0, (), b'', b'A\0BCDzeer', b'', False)]
        sources += [('cxx-string-' + mode, ROOT / 'tools/native-c/tests/cxx-string-failure.cpp', 134, (mode,), b'', b'armed\n', b'', False)
                    for mode in ('allocation', 'overflow', 'null')]
        sources += [('wchar-contract', ROOT / 'tools/native-c/tests/wchar-contract.c', 0, (), b'', b'', b'', False)]
        sources += [('termios-contract', ROOT / 'tools/native-c/tests/termios.c', 0, (), b'', b'', b'', False)]
        sources += [('dlfcn-contract', ROOT / 'tools/native-c/tests/dlfcn.c', 0, (), b'', b'', b'', False)]
        sources += [('epoll-contract', ROOT / 'tools/native-c/tests/epoll.c', 0, (), b'', b'', b'', False)]
        sources += [('pthread-' + mode, ROOT / 'tools/native-c/tests/pthread.c', 0, (mode,),
                     b'', b'', b'', False)
                    for mode in ('basic', 'exit', 'detached', 'attr')]
        sources += [('inet-socket-contract', ROOT / 'tools/native-c/tests/inet-socket.c', 0,
                     (), b'', b'', b'', False),
                    ('inet-socket-reap', ROOT / 'tools/native-c/tests/inet-socket.c', 0,
                     ('leak',), b'', b'', b'', False)]
        sources += [('epoll-lifecycle', ROOT / 'tools/native-c/tests/epoll-lifecycle.c', 0,
                     (), b'', b'', b'', False)]
        sources += [('signal-contract', ROOT / 'tools/native-c/tests/signal.c', 0,
                     (), b'', b'', b'', False)]
        sources += [('signal-' + mode, ROOT / 'tools/native-c/tests/signal.c', code,
                     (mode,), b'', b'', b'', False) for mode, code in [('fatal', 138), ('forged', 139), ('handler-exit', 31), ('bad-stack', 139)]]
        sources += [('pthread-abi', ROOT / 'tools/native-c/tests/pthread-abi.c', 0,
                     (), b'', b'', b'', False)]
        sources += [('poll-contract', ROOT / 'tools/native-c/tests/poll.c', 0, (), b'', b'', b'', False)]
        sources += [('poll-socket-contract', ROOT / 'tools/native-c/tests/poll-socket.c', 0,
                     (), b'', b'', b'', False)]
        sources += [('math-contract', ROOT / 'tools/native-c/tests/math.c', 0, (), b'', b'', b'', False)]
        sources += [('locale-contract', ROOT / 'tools/native-c/tests/locale.c', 0, (), b'', b'', b'', False),
                    ('locale-environment', ROOT / 'tools/native-c/tests/locale.c', 0, ('environment',), b'', b'', b'', False)]
        sources += [('langinfo-contract', ROOT / 'tools/native-c/tests/langinfo.c', 0, (), b'', b'', b'', False)]
        sources += [
            ('stdout', ROOT / 'tools/native-c/tests/stdout.c', 0, (), b'', b'[NATIVE-C] stdout OK\n', b'', False),
            ('stdio-full', ROOT / 'tools/native-c/tests/stdio.c', 0, (WORK,), b'Q\n', STDOUT, b'stdio: stderr 0007\n', False),
            ('open-unlink', ROOT / 'tools/native-c/tests/open-unlink.c', 0, (WORK,), b'', b'', b'', False),
            ('exit-cleanup', ROOT / 'tools/native-c/tests/exit-cleanup.c', 0, (), b'', b'', b'', False),
        ]
        # Nine Phase 5 contracts are separate from the historical API matrix
        # and the four previously added focused fixtures. Probe sources and
        # expected behavior of the historical cases remain unchanged.
        sources += [(name, ROOT / 'tools/native-c/tests/unistd.c', code, (mode,),
                     b'', stdout, stderr, False)
                    for name, mode, code, stdout, stderr in (
                        ('write-basic', 'basic', 0, b'write OK\n', b'stderr OK\n'),
                        ('write-zero', 'zero', 0, b'', b''),
                        ('write-short', 'short', 0, b'', b''),
                        ('write-errors', 'errors', 0, b'', b''),
                        ('atexit-order', 'order', 0, b'CBA', b''),
                        ('atexit-register', 'register', 0, b'BCA', b''),
                        ('atexit-capacity', 'capacity', 0, b'x' * 32, b''),
                        ('atexit-exit', 'exit', 27, b'A', b''),
                        ('atexit-raw-exit', 'raw-exit', 29, b'', b''),
                    )]
        sources += [('random-' + mode, random_fixture, 0, (mode,), b'', b'', b'', False)
                    for mode in ('basic', 'zero', 'errors', 'guards')]
        sources += [('time-' + mode, ROOT / 'tools/native-c/tests/time.c', 0, (mode,),
                     b'', None if mode in ('clocks', 'sleep') else b'', b'', False)
                    for mode in ('clocks', 'errors', 'zero', 'sleep')]
        sources += [('mman-' + mode, ROOT / 'tools/native-c/tests/mman.c', 0, (mode,),
                     b'', b'', b'', False) for mode in ('subset', 'fixed', 'bounds')]
        sources += [('dirent-' + mode, ROOT / 'tools/native-c/tests/dirent.c', 0,
                     (mode,) if mode == 'root' else
                     (mode, '/tmp/native-c-dirent' if mode == 'entries' else WORK),
                     b'', b'', b'', False)
                    for mode in ('root', 'errors', 'entries')]
        for name, src, code, args, input, stdout, stderr, historical in sources:
            elf = out / (name + '.elf')
            compiler = ([sys.executable, ROOT / 'tools/native-c/cc.py', '-std=c11', '-Wall', '-Wextra', '-Werror']
                        if src.suffix == '.c' else ['clang++', '--target=x86_64-unknown-none-elf',
                            '-nostdinc', '-I' + str(ROOT / 'sunlight-libc/include'), '-nostdlib', '-fuse-ld=lld'])
            if not historical and src.suffix == '.cpp':
                compiler = [sys.executable, ROOT / 'tools/native-c/cxx.py', '-Wall', '-Wextra', '-Werror']
                if name in ('cxx-backend', 'cxx-string-backend'):
                    compiler += ['-Wl,--wrap=write']
                if name in ('cxx-string', 'cxx-exception-class') or src.name in ('cxx-runtime-execution.cpp', 'cxx-local-static.cpp'):
                    compiler += ['-Wl,--wrap=malloc,--wrap=free']
                if src.name == 'cxx-runtime-execution.cpp':
                    compiler += ['-Wl,--wrap=pthread_create']
                if src.name == 'cxx-string-failure.cpp':
                    compiler += ['-Wl,--wrap=malloc']
            if command('build-' + name, [*compiler, src, '-o', elf]):
                link_failure = 'undefined symbol' in (out / ('build-' + name + '.log')).read_text()
                rows.append(dict(probe=name, historical=historical, build='FAIL', guest_run='NOT_RUN',
                                 acceptance='LINK_FAIL' if link_failure else 'BUILD_FAIL', exit='', expected_exit=code,
                                 detail='BUILD_FAILED: no executable to run', sha256=''))
            else:
                command('elf-' + name, ['llvm-readelf', '-h', '-l', elf])
                add(name, elf, code, args, input, stdout, stderr, historical,
                    environment=(b'TMPDIR=/tmp', b'LANG=en_US.UTF-8') if name == 'locale-environment' else (b'TMPDIR=/tmp',))
    else:
        add('elf', opts.elf, opts.expect_exit, opts.arg,
            opts.stdin_file.read_bytes() if opts.stdin_file else b'',
            environment=(b'TMPDIR=/tmp', *(entry.encode() for entry in opts.env)))

    support_target = ROOT / 'target/native-c-guest/support'
    kernel_target = ROOT / 'target/native-c-guest/kernel'
    support_env = dict(os.environ, RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static')
    for key in ('CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR'):
        support_env.pop(key, None)
    support_packages = ['-p', 'sunlight-init', '-p', 'rand_service']
    if opts.net:
        support_packages += ['-p', 'sunlight-net-server']
    if command('support-build', ['cargo', 'build', *support_packages,
                                 '--release', '--target', 'x86_64-unknown-none',
                                 '--target-dir', support_target], env=support_env):
        return 1
    manifest = out / 'manifest.rs'
    net_elf = '&[]'
    service_elf = '&[]'
    wants_network = 'false'
    if opts.net:
        wants_network = 'true'
        net_elf = 'include_bytes!(' + json.dumps(
            str(support_target / 'x86_64-unknown-none/release/net_server')) + ')'
        if opts.service:
            service_elf = 'include_bytes!(' + json.dumps(str(opts.service.resolve())) + ')'
    manifest.write_text('static LOCALE_CONF: &[u8] = ' + rust_bytes(opts.locale_conf.read_bytes() if opts.locale_conf else b'') + ';\n' +
                        'static INIT_ELF: &[u8] = include_bytes!(' + json.dumps(str(support_target / 'x86_64-unknown-none/release/sunlight-init')) + ');\n' +
                        'static RAND_ELF: &[u8] = include_bytes!(' + json.dumps(str(support_target / 'x86_64-unknown-none/release/rand_service')) + ');\n' +
                        'pub const WANTS_NETWORK: bool = ' + wants_network + ';\n' +
                        'static NET_ELF: &[u8] = ' + net_elf + ';\n' +
                        'static SERVICE_ELF: &[u8] = ' + service_elf + ';\n' +
                        'static CASES: &[Case] = &[\n' + ''.join(
        'Case { name: ' + json.dumps(c['name']) + ', elf: include_bytes!(' + json.dumps(str(c['elf'])) +
        '), args: &[' + ','.join(rust_bytes(a.encode()) for a in c['args']) + '], input: ' +
        rust_bytes(c['input']) + ', environment: &[' + ','.join(rust_bytes(e) for e in c['environment']) + '] },\n' for c in cases) + '];\n')
    env = dict(os.environ, SUNLIGHT_NATIVE_C_MANIFEST=str(manifest))
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR'):
        env.pop(key, None)
    print(f'Building isolated native C guest ({len(cases)} ELFs, {opts.cpus} CPUs)', flush=True)
    # Serialize this runner's manifest builds through copying the completed ELF;
    # QEMU and ISO creation subsequently use only this run's private snapshot.
    kernel = out / 'kernel.elf'
    with (ROOT / 'target/native-c-guest-build.lock').open('w') as build_lock:
        fcntl.flock(build_lock, fcntl.LOCK_EX)
        if command('kernel-build', ['cargo', 'build', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel',
                                   '--target', 'x86_64-unknown-none', '--target-dir', kernel_target,
                                   '--no-default-features', '--features', 'native_c_guest'], env=env):
            return 1
        shutil.copy2(kernel_target / 'x86_64-unknown-none/debug/sunlight-kernel', kernel)
    (out / 'kernel-sha256.log').write_text(hashlib.sha256(kernel.read_bytes()).hexdigest() + '\n')
    # make_hybrid_iso.sh accepts a separate staging root. Never use its ordinary
    # target/iso_root or attach the developer's sunlightos_disk.img.
    stage = out / 'media'
    stage.mkdir()
    (stage / 'limine.conf').write_bytes((ROOT / 'limine.conf').read_bytes())
    iso = out / 'guest.iso'
    if command('iso-build', ['bash', 'tools/make_hybrid_iso.sh',
            kernel, iso, ROOT / 'target/limine', stage]):
        return 1
    serial = out / 'serial.log'
    argv = ['qemu-system-x86_64', '-accel', 'tcg', '-cpu', 'max', '-m', '2048M', '-smp', str(opts.cpus),
            '-cdrom', str(iso), '-boot', 'd', '-display', 'none', '-monitor', 'none',
            '-serial', 'file:' + str(serial),
            '-device', 'virtio-rng-pci,disable-modern=on',
            '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04', '-no-reboot']
    if opts.net:
        netdev = 'user,id=net0' + (',net=' + opts.net_subnet if opts.net_subnet else '')
        argv += ['-netdev', netdev,
                 '-device', 'virtio-net-pci,netdev=net0,disable-modern=on',
                 '-object', f'filter-dump,id=dump,netdev=net0,file={out / "network.pcap"}']
    else:
        argv += ['-nic', 'none']
    print('Booting disposable TCG guest', flush=True)
    rc = command('qemu', argv, timeout=opts.timeout)
    output = serial.read_text(errors='replace') if serial.exists() else ''
    if opts.net and (out / 'network.pcap').is_file():
        (out / 'network-summary.log').write_text(summarize_pcap(out / 'network.pcap'))
    fault_pattern = r'KERNEL[ _]PANIC|\[EXCEPTION\]|\[FAULT\]|\[INT\] (?:Divide Error|Invalid Opcode|Double Fault|General Protection)|Unknown syscall|Unhandled syscall'
    fault = re.search(fault_pattern, output, re.I)
    (out / 'faults.log').write_text(''.join(line + '\n' for line in output.splitlines()
                                           if re.search(fault_pattern, line, re.I)))
    done = f'[NATIVE-C-DONE] cases={len(cases)}' in output and rc == 33 and not fault
    results = {}
    for number, case in enumerate(cases):
        row = next(r for r in rows if r['probe'] == case['name'])
        observed = re.findall(rf'^\[NATIVE-C-RESULT\] case={number} exit=(-?\d+) stdout=(\d+) stderr=(\d+) clean=([01]) .*$', output, re.M)
        streams = {}
        for fd, kind in ((1, 'stdout'), (2, 'stderr')):
            data = b''.join(bytes.fromhex(x) for x in re.findall(
                rf'^\[NATIVE-C-STREAM\] case={number} fd={fd} hex=([0-9a-f]+)$', output, re.M))
            (out / (case['name'] + '.' + kind + '.txt')).write_bytes(data)
            streams[kind] = data
        if len(observed) == 1:
            code, nout, nerr, clean = map(int, observed[0])
            result_ok = (clean == 1 and len(streams['stdout']) == nout and len(streams['stderr']) == nerr and
                         (case['expected'] is None or code == case['expected']) and
                         all(case[k] is None or streams[k] == case[k] for k in streams))
            reap = re.search(rf'^\[NATIVE-C-REAP-END\] case={number} exit={code} user_frames=([1-9]\d*) page_tables=([1-9]\d*) owned_frames=0 fds=0 stack=0$', output, re.M)
            result_ok = result_ok and bool(reap) and not fault
            row.update(guest_run='PASS' if result_ok else 'FAIL', acceptance='PASS' if result_ok else 'FAIL', exit=code,
                       detail='exit/output/reap/physical-frame checks' if result_ok else 'guest contract or infrastructure failed')
            results[case['name']] = dict(exit=code, stdout=streams['stdout'].decode(errors='replace'),
                                         stderr=streams['stderr'].decode(errors='replace'), pass_=result_ok)
        else:
            row.update(guest_run='FAIL', detail='no unique completed guest result; inspect serial.log and qemu.log')
        print(f"{row['probe']}: {row['acceptance']} exit={row['exit']} expected={row['expected_exit']}", flush=True)
        for kind, data in streams.items():
            if data:
                print(f"guest {case['name']} {kind}: {data!r}", flush=True)
    if opts.suite:
        snapshots = []
        for name in ('random-service-before', 'random-service-after'):
            observed = results.get(name, {})
            match = re.fullmatch(r'rand service: requests=(\d+) bytes=(\d+)\n', observed.get('stdout', ''))
            if observed.get('pass_') and match:
                snapshots.append(tuple(map(int, match.groups())))
        service_ok = (len(snapshots) == 2 and results.get('getrandom', {}).get('pass_') and
                      tuple(b - a for a, b in zip(snapshots[0], snapshots[1])) == (1, 16))
        (out / 'random-service.log').write_text(
            f'snapshots={snapshots}\nexpected_delta=requests:1,bytes:16\n'
            f'unchanged_getrandom_service_backing={"PASS" if service_ok else "FAIL"}\n')
        if not service_ok:
            done = False
            if 'getrandom' in results:
                results['getrandom']['pass_'] = False
            for row in rows:
                if row['probe'] == 'getrandom' and row['build'] == 'PASS':
                    row.update(acceptance='FAIL', detail='random-service counter verification failed')
    with (out / 'results.csv').open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=rows[0])
        writer.writeheader()
        writer.writerows(rows)
    (out / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    history = [r for r in rows if r['historical']]
    if history:
        print(f"Historical 21: PASS={sum(r['acceptance'] == 'PASS' for r in history)} "
              f"guest_FAIL={sum(r['guest_run'] == 'FAIL' for r in history)} "
              f"BUILD_FAIL={sum(r['acceptance'] == 'BUILD_FAIL' for r in history)} "
              f"LINK_FAIL={sum(r['acceptance'] == 'LINK_FAIL' for r in history)} "
              f"guest_NOT_RUN={sum(r['guest_run'] == 'NOT_RUN' for r in history)}")
    # Known missing APIs are recorded acceptance failures, not an infrastructure
    # failure of this focused phase. Every built guest must actually pass.
    if not done or any(r['guest_run'] != 'PASS' for r in rows if r['build'] == 'PASS'):
        return 1
    if not opts.suite and opts.expect_exit is None:
        code = int(rows[0]['exit'])
        return code if 0 <= code <= 255 else 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
