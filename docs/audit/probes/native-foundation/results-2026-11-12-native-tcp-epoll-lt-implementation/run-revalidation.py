import hashlib,json,subprocess,sys,shlex,time
from pathlib import Path
out=Path(__file__).resolve().parent/'final'
archive=Path('target/native-c/sysroot/lib/libsunlight_native_c.a')
rows=[]
for cpu in (2,4):
    name='revalidation-'+str(cpu)
    args=[sys.executable,'tools/native-c/guest.py',str(out/'tcp-epoll-revalidation.elf'),'--net','--cpus',str(cpu),'--expect-exit','0','--timeout','180','--out',str(out/name)]
    with (out/(name+'.log')).open('w') as f:
        f.write('$ '+shlex.join(args)+'\n');f.flush()
        p=subprocess.run(args,stdout=f,stderr=subprocess.STDOUT,timeout=600)
        f.write('\nexit='+str(p.returncode)+'\n')
    rows.append(dict(argv=args,exit=p.returncode,archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest()))
    (out/'revalidation-ledger.json').write_text(json.dumps(rows,indent=2)+'\n')
