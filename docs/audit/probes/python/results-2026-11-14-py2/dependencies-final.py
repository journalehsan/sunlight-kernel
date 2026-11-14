import pathlib,json,subprocess,concurrent.futures,hashlib
E=pathlib.Path(__file__).resolve().parent;B=pathlib.Path('/tmp/sunlight-py2-cpython/cross-static');p=json.loads((E/'cpython-compilation-matrix.json').read_text())
def one(u):
 if not u['attempted']:return u
 a=u['command'].copy();o=a.index('-o');a=a[:o]+a[o+2:];a.remove('-c');a+=['-M','-MG'];r=subprocess.run(a,cwd=B,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True);name='qualified-'+u['object'].replace('/','_')+'.d';(E/name).write_text(r.stdout);u['source_sha256']=hashlib.sha256((pathlib.Path(u['source']) if pathlib.Path(u['source']).is_absolute() else B/u['source']).read_bytes()).hexdigest();u['transitive_dependencies']={'status':'PASS' if r.returncode==0 else 'FAIL','manifest':name,'command':a,'diagnostics':r.stderr};return u
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:p['units']=list(pool.map(one,p['units']))
(E/'cpython-compilation-matrix.json').write_text(json.dumps(p,indent=2))
