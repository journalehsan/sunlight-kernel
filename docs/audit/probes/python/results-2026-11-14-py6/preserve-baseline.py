from pathlib import Path
import json,hashlib,subprocess,tempfile,shutil,difflib
R=Path.cwd();E=Path(__file__).resolve().parent;B=json.loads((E/'source-baseline.json').read_text())['files']
sha=lambda b:hashlib.sha256(b).hexdigest()
changed=[n for n,h in B.items() if (R/n).exists() and sha((R/n).read_bytes())!=h]
T=Path(tempfile.mkdtemp(prefix='py6-incoming-reconstruction-'));candidates={}
for phase in [4,5]:
 P=E.parent/f'results-2026-11-14-py{phase}';D=T/str(phase);shutil.copytree(P/'incoming',D)
 p=subprocess.run(['patch','-p1','-i',str(P/f'py{phase}-production.patch')],cwd=D,capture_output=True,text=True)
 (E/f'baseline-reconstruct-py{phase}.log').write_text(p.stdout+p.stderr)
 for f in D.rglob('*'):
  if f.is_file():candidates.setdefault(str(f.relative_to(D)),[]).append(f.read_bytes())
for n in changed:
 p=R/n;b=p.read_bytes();options=candidates.get(n,[])
 g=subprocess.run(['git','show','HEAD:'+n],capture_output=True)
 if g.returncode==0:options.append(g.stdout)
 s=b.decode()
 if n=='sunlight-libc/src/lib.rs':
  a=s.index('/// Authoritative existing-object canonical path');z=s.index('/// Change the current process CWD',a);s=s[:a]+s[z:];s=s.replace('/// Process locale selection and environment/configuration bridge.\n#[cfg(any(test, all(feature = "native-std", target_os = "sunlightos")))]','/// Process locale selection and environment/configuration bridge.\n#[cfg(any(test, all(feature = "native-c", target_os = "sunlightos")))]');options.append(s.encode())
 if n=='sunlight-libc/include/stdlib.h':
  a=s.index('/* Existing-object canonical query');z=s.index('/* Private per-call conversion',a);options.append((s[:a]+s[z:]).encode())
 if n=='sunlight-libc/include/limits.h':options.append(s.replace('\n#define PATH_MAX 256','').encode())
 if n=='sunlight-libc/include/errno.h':options.append(s.replace('#define ENAMETOOLONG 36\n','').encode())
 if n=='sunlight-libc/src/c_file.rs':options.append(s[:s.index('\n/// Native realpath')].encode())
 original=next((v for v in options if sha(v)==B[n]),None)
 if original is None:raise RuntimeError('Cannot reconstruct original '+n)
 dest=E/'incoming'/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(original)
 print('verified incoming',n)
patch=''
for n in changed:
 original=(E/'incoming'/n).read_text();final=(R/n).read_text();patch+=''.join(difflib.unified_diff(original.splitlines(True),final.splitlines(True),fromfile='a/'+n,tofile='b/'+n))
new=['sunlight-fs/src/vfs/lookup.rs']
for n in new:patch+=''.join(difflib.unified_diff([], (R/n).read_text().splitlines(True),fromfile='/dev/null',tofile='b/'+n))
(E/'py6-production.patch').write_text(patch)
(E/'modified-production-files.json').write_text(json.dumps([dict(path=n,incoming_sha256=B.get(n),sha256=sha((R/n).read_bytes())) for n in changed+new],indent=2))
shutil.rmtree(T)
