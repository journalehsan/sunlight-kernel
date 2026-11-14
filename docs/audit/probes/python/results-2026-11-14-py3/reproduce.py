import pathlib,json,subprocess,concurrent.futures
E=pathlib.Path(__file__).resolve().parent;P=E.parent/'results-2026-11-14-py2';x=json.loads((P/'cpython-compilation-matrix.json').read_text())
def one(u):
 r={k:u.get(k) for k in ['object','source','command','status','first_error','mandatory']};r['py2_result']=r.pop('status')
 if not u['attempted']:
  r.update(status='NOT_RUN',reason='PY-2 unattempted; no diagnosis inferred');return r
 p=subprocess.run(u['command']+['-ferror-limit=0'],cwd='/tmp/sunlight-py2-cpython/cross-static',capture_output=True,text=True);s=p.stdout+p.stderr;(E/('baseline-'+u['object'].replace('/','_')+'.log')).write_text(s);r.update(status='PASS' if p.returncode==0 else 'FAIL',first_error=next((l for l in s.splitlines() if 'error:' in l),None));return r
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:r=list(pool.map(one,[u for u in x['units'] if u['status']!='PASS']))
(E/'py2-failures-reproduced.json').write_text(json.dumps(r,indent=2));print([(u['object'],u['status']) for u in r])
