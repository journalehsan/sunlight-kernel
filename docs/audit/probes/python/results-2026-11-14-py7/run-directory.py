import pathlib,subprocess,json,hashlib,time
E=pathlib.Path(__file__).resolve().parent;R=pathlib.Path.cwd(); hashes=json.loads((E/'validated-source.json').read_text()); rows=[]
for cpu in [1,2,4]:
 cmd=['env','SUNLIGHT_TRUST_LEGACY_STATE=1','python3','tools/native-c/guest.py','target/py7-directory.elf','--expect-exit','0','--fat-fixture',str(E/'fat-fixture.img'),'--cpus',str(cpu),'--timeout','150','--out',str(E/('directory-native-cpu-'+str(cpu)))]
 before=all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items());t=time.monotonic()
 with (E/('directory-native-cpu-'+str(cpu)+'.log')).open('w') as f:
  f.write(repr(cmd)+'\n');f.flush();p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 rows.append(dict(cpu=cpu,command=cmd,exit=p.returncode,seconds=time.monotonic()-t,source_matches=before and all(hashlib.sha256((R/p).read_bytes()).hexdigest()==h for p,h in hashes.items())))
 (E/'directory-executions.json').write_text(json.dumps(rows,indent=2))
