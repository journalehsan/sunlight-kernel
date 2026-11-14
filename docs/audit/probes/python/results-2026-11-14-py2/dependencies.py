import pathlib,json,subprocess,concurrent.futures
E=pathlib.Path(__file__).resolve().parent;R=pathlib.Path.cwd();H=pathlib.Path('/tmp/sunlight-py2-baseline-headers');resource=subprocess.check_output(['clang','-print-resource-dir'],text=True).strip();p=json.loads((E/'failure-inventory-before.json').read_text())
def one(u):
 if not u['attempted']:return u
 a=u['command'][2:];o=a.index('-o');a=a[:o]+a[o+2:];a.remove('-c')
 cmd=['clang','--target=x86_64-unknown-none-elf','-march=x86-64','-mno-red-zone','-mno-mmx','-mno-avx','-mno-avx2','-ffreestanding','-fno-pic','-fno-pie','-fno-stack-protector','-nostdinc','-isystem',resource+'/include','-I'+str(H),'--sysroot='+str(R/'target/native-c/sysroot'),*a,'-M','-MG']
 r=subprocess.run(cmd,cwd='/tmp/sunlight-py2-cpython/cross-static',stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 name='before-'+u['object'].replace('/','_')+'.d';(E/name).write_text(r.stdout);u['transitive_dependencies']={'status':'PASS' if r.returncode==0 else 'FAIL','manifest':name,'command':cmd,'diagnostics':r.stderr};u['mandatory']=not u['source'].endswith('/unicodedata.c');u['source_sha256']=__import__('hashlib').sha256(pathlib.Path(u['source']).read_bytes()).hexdigest() if pathlib.Path(u['source']).is_absolute() else None;return u
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:p['units']=list(pool.map(one,p['units']))
(E/'failure-inventory-before.json').write_text(json.dumps(p,indent=2))
