#!/usr/bin/env python3
"""Writes only below docs/audit. Host execution is NEVER guest evidence.
Optional HELIOS_RUNNER / NATIVE_RUNNER accepts an ELF path and must print a
SunlightOS guest transcript, exit nonzero on probe failure, and use isolated,
read-only/snapshot guest media. No existing VM is contacted by this script.
"""
import argparse, csv, json, os, pathlib, shlex, shutil, subprocess, sys
BASE = pathlib.Path(__file__).resolve().parent
ROOT = BASE.parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--targets', nargs='+', choices=('host', 'helios', 'native'),
                    default=('host', 'helios', 'native'))
parser.add_argument('--out', type=pathlib.Path, help='new output directory below probes/')
options = parser.parse_args()
OUT = options.out.resolve() if options.out else BASE / 'results'
if options.out and not OUT.is_relative_to(BASE):
    parser.error('--out must be below docs/audit/probes/')
OUT.mkdir(parents=True, exist_ok=not bool(options.out))
if options.out:
    (OUT / '.gitignore').write_text((BASE / 'results/.gitignore').read_text())
def run(args, name, timeout=30):
    try:
        p = subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, timeout=timeout)
        code, output = p.returncode, p.stdout
    except (OSError, subprocess.TimeoutExpired) as e:
        code, output = 125, str(e)
    (OUT / (name + '.log')).write_text('$ ' + shlex.join(args) + '\n' + output + '\nexit=' + str(code) + '\n')
    return code
inventory = [ ['git','rev-parse','HEAD'], ['git','status','--short'],
 ['python3','--version'], ['rustc','-Vv'], ['clang','--version'], ['gcc','--version'],
 ['ld.lld','--version'], ['musl-gcc','-print-sysroot'], ['musl-gcc','-v'],
 ['musl-gcc','-print-file-name=libc.a'], ['g++','-print-file-name=libstdc++.a'],
 ['sh','-c','command -v x86_64-linux-musl-g++ musl-g++ clang++ g++ qemu-system-x86_64; ls /usr/lib/musl /usr/include/x86_64-linux-musl'],
 ['rg','--files','-g','*python*','-g','*libc++*','-g','*libunwind*','-g','*sysroot*','-g','*rartui*'],
 ['rg','-n','dlopen|dlsym|pthread_create|setlocale|nl_langinfo|__cxa_throw|__cxa_atexit','sunlight-libc','tools/native-std','third_party'],
 ['rg','-n','/proc/self|/dev/null|/dev/urandom|/dev/tty','kernel/src','sunlight-fs/src','services/vfs_server/src'],
 ['rg','-n','symlink|readlink|mtime|eq_ignore_ascii_case','sunlight-fs/src','sunlight-fat/src'],
 ['rg','-n','rartui|ratatui|tokio','helios-note/Cargo.toml','rappid-rabbit/Cargo.toml','sunlight-tui'],
 ['rg','-n','PT_INTERP|classify_personality|ET_DYN','sunlight-elf/src/lib.rs'],
 ['rg','-n','SYS_(FTRUNCATE|FSYNC|SELECT|STATX|MREMAP|ALARM|GETRLIMIT|PRLIMIT64|RT_SIGRETURN)','compat-linux/src/lib.rs','compat-linux/src/abi.rs'],
 ['rg','-n','signal|fork|mmap|thread|net|process','docs/native-std-post-implementation-audit.md'],
 ['rg','-n','SYS_|test|PASS','tools/helios-probes/linux-thread-probe.S','tools/tests/helios_thread_probe.expected']]
for i, cmd in enumerate(inventory): run(cmd, 'inventory-%02d'%i)
run(['clang','--target=x86_64-unknown-none-elf','-std=c11','-ffreestanding','-c',str(BASE/'compiler.c'),'-o',str(OUT/'compiler.o')], 'native-compiler-only')
run(['sh','-c','cat /usr/lib/musl/lib/musl-gcc.specs; ls /usr/lib/musl/lib'], 'musl-layout')
rows=[]
for src in sorted(x for x in BASE.glob('*.c') if x.name != 'compiler.c') + sorted(BASE.glob('*.cpp')):
    cpp = src.suffix == '.cpp'
    for target in options.targets:
        elf = OUT / (target + '-' + src.stem)
        if target == 'host': cmd = ['g++' if cpp else 'cc',str(src),'-o',str(elf),'-pthread','-lm','-ldl']
        elif target == 'helios':
            compiler = os.environ.get('HELIOS_CXX' if cpp else 'HELIOS_CC', 'x86_64-linux-musl-g++' if cpp else 'musl-gcc')
            cmd = shlex.split(compiler)+[str(src),'-static','-no-pie','-o',str(elf),'-pthread','-lm','-ldl']
        else:
            compiler = os.environ.get('NATIVE_CXX' if cpp else 'NATIVE_CC', 'clang++ --target=x86_64-unknown-none-elf' if cpp else shlex.join([sys.executable, str(ROOT/'tools/native-c/cc.py')]))
            # The C driver supplies canonical startup/libs after build_native_c.sh.
            # C++ remains a separate, unavailable native runtime configuration.
            resource = subprocess.check_output(['clang','-print-resource-dir'], text=True).strip()
            cmd = shlex.split(compiler)+['-isystem',resource+'/include','-nostdinc','-I'+str(ROOT/'sunlight-libc/include'),str(src),'-o',str(elf),'-nostdlib','-fuse-ld=lld']
        key = target+'-'+src.stem
        rc=run(cmd,key+'-build')
        record=dict(target=target,probe=src.name,build='PASS' if rc==0 else 'FAIL',host_run='NOT_RUN',guest_run='UNKNOWN')
        if rc==0:
            if target in ('host','helios'):
                record['host_run']='PASS' if run([str(elf)],key+'-host-run')==0 else 'FAIL'
            if target!='host':
                runner=os.environ.get(target.upper()+'_RUNNER')
                if target=='helios':
                    data=bytearray(elf.read_bytes()); data[7]=3; data[9:15]=b'HLNX01'; elf.write_bytes(data)
                if runner:
                    record['guest_run']='PASS' if run(shlex.split(runner)+[str(elf)],key+'-guest-run',120)==0 else 'FAIL'
        rows.append(record)
with (OUT/'results.csv').open('w') as f:
    w=csv.DictWriter(f,fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
print(json.dumps(rows,indent=2))
