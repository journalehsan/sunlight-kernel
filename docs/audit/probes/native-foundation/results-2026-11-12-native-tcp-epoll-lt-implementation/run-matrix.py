#!/usr/bin/env python3
import hashlib, json, os, signal, subprocess, sys, time
from pathlib import Path
ROOT=Path.cwd(); OUT=Path(__file__).resolve().parent/"final"; OUT.mkdir(exist_ok=False)
archive=ROOT/'target/native-c/sysroot/lib/libsunlight_native_c.a'
ledger=[]
def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def run(name, args, timeout=900):
    before=digest(archive)
    argv=list(map(str,args)); log=OUT/(name+'.log')
    start=time.time()
    with log.open('w') as f:
        import shlex
        f.write('$ '+shlex.join(argv)+'\n'); f.flush()
        p=subprocess.Popen(argv,cwd=ROOT,stdout=f,stderr=subprocess.STDOUT,start_new_session=True)
        try: code=p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid,signal.SIGTERM)
            try: p.wait(timeout=5)
            except subprocess.TimeoutExpired: os.killpg(p.pid,signal.SIGKILL); p.wait()
            code=124
        f.write('\nexit='+str(code)+'\n')
    ledger.append(dict(name=name,argv=argv,exit=code,seconds=round(time.time()-start,2),archive_before=before,archive_after=digest(archive)))
    (OUT/'matrix-ledger.json').write_text(json.dumps(ledger,indent=2)+'\n')
    print(name,code,flush=True)
    return code
py=sys.executable
run('build-archive',['bash','tools/build_native_c.sh'],1800)
lt=OUT/'tcp-epoll-lt-final2.elf'
if run('build-lt',[py,'tools/native-c/cc.py','-std=c11','-Wall','-Wextra','-Werror','-Wl,-t','tools/native-c/tests/tcp-epoll-lt.c','-o',lt],300): sys.exit(1)
run('lt-2-final',[py,'tools/native-c/guest.py',lt,'--expect-exit','0','--net','--cpus','2','--timeout','180','--out',OUT/'lt-2-final'])
run('lt-4-final',[py,'tools/native-c/guest.py',lt,'--expect-exit','0','--net','--cpus','4','--timeout','180','--out',OUT/'lt-4-final'])
for cpu,suffix in [(2,'2'),(4,'4'),(2,'2-repeat')]:
    run('epoll-external-'+suffix,[py,'tools/native-c/run_epoll_tcp.py','--mode','external','--cpus',str(cpu),'--skip-build','--out',OUT/('epoll-external-'+suffix)])
for mode,cpu,suffix in [('external',2,'2'),('external',4,'4'),('external',2,'2-repeat'),('loopback',2,'2'),('loopback',4,'4')]:
    name='epoll-pressure-'+mode+'-'+suffix
    run(name,[py,'tools/native-c/run_epoll_pressure.py','--mode',mode,'--cpus',str(cpu),'--skip-build','--out',OUT/name])
for abi in ['poll','socket']:
    name='regress-'+abi+'-abi'
    run(name,[py,'tools/native-c/tests/run_'+abi+'.py','--out',OUT/name])
for fixture in ['epoll','epoll-lifecycle','tcp-echo-client','tcp-echo-service']:
    run('build-'+fixture,[py,'tools/native-c/cc.py','-std=c11','-Wall','-Wextra','-Werror','-Wl,-t','tools/native-c/tests/'+fixture+'.c','-o',OUT/(fixture+'.elf')],300)
for cpu in [2,4]:
    for fixture in ['epoll','epoll-lifecycle']:
        name='regress-'+fixture+'-'+str(cpu)
        run(name,[py,'tools/native-c/guest.py',OUT/(fixture+'.elf'),'--expect-exit','0','--cpus',str(cpu),'--out',OUT/name,'--timeout','120'])
    name='regress-loopback-echo-'+str(cpu)
    run(name,[py,'tools/native-c/guest.py',OUT/'tcp-echo-client.elf','--expect-exit','0','--service',OUT/'tcp-echo-service.elf','--net','--cpus',str(cpu),'--out',OUT/name,'--timeout','180'])
    for mode in ['loopback','external']:
        name='regress-poll-'+mode+'-'+str(cpu)
        run(name,[py,'tools/native-c/run_poll_tcp.py','--mode',mode,'--cpus',str(cpu),'--skip-build','--out',OUT/name])
        name='regress-pressure-'+mode+'-'+str(cpu)
        run(name,[py,'tools/native-c/run_poll_pressure.py','--mode',mode,'--cpus',str(cpu),'--skip-build','--out',OUT/name])
    for mode in ['echo','stall']:
        name='regress-external-'+mode+'-'+str(cpu)
        run(name,[py,'tools/native-c/run_external_tcp.py','--mode',mode,'--cpus',str(cpu),'--skip-build','--out',OUT/name])
    name='suite-'+str(cpu)
    run(name,[py,'tools/native-c/guest.py','--suite','--cpus',str(cpu),'--timeout','360','--out',OUT/name],1500)
run('regress-kernel-check',['cargo','check','-p','sunlight-kernel'],300)
run('regress-net-ipc-server-check',['cargo','check','-p','sunlight-net','-p','sunlight-ipc','-p','sunlight-net-server'],300)
run('regress-libc-check',['cargo','check','-p','sunlight-libc','--features','native-c'],300)
run('diff-check',['git','diff','--check'],30)
(OUT/'artifact-hashes.json').write_text(json.dumps({str(p.relative_to(ROOT)):digest(p) for p in [archive,ROOT/'target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a',*OUT.rglob('*.elf')]},indent=2)+'\n')
