import hashlib,json,subprocess,sys,shlex
from pathlib import Path
out=Path(__file__).resolve().parent/'final'
rows=[]
for cpu in (2,4):
    name='inet-lt-'+str(cpu)
    args=[sys.executable,'tools/native-c/guest.py',str(out/'inet-socket-epoll-lt.elf'),'--cpus',str(cpu),'--expect-exit','0','--timeout','180','--out',str(out/name)]
    with (out/(name+'.log')).open('w') as f:
        f.write('$ '+shlex.join(args)+'\n');f.flush()
        p=subprocess.run(args,stdout=f,stderr=subprocess.STDOUT,timeout=600)
        f.write('\nexit='+str(p.returncode)+'\n')
    rows.append(dict(argv=args,exit=p.returncode,archive_sha256=hashlib.sha256(Path('target/native-c/sysroot/lib/libsunlight_native_c.a').read_bytes()).hexdigest()))
    (out/'inet-companion-ledger.json').write_text(json.dumps(rows,indent=2)+'\n')
