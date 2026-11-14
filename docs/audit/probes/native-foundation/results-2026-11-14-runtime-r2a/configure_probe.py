import pathlib,hashlib,tarfile,subprocess,json,os,shlex,difflib
r=pathlib.Path.cwd(); e=pathlib.Path(__file__).resolve().parent; b=pathlib.Path('/tmp/sunlight-r2a-cpython'); b.mkdir(exist_ok=False)
a=pathlib.Path('/tmp/sunlight-r1-Python-3.13.0.tar.xz'); digest=hashlib.sha256(a.read_bytes()).hexdigest(); assert digest=='086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d'
with tarfile.open(a) as t:t.extractall(b,filter='data')
src=b/'Python-3.13.0'; r1=pathlib.Path('/tmp/sunlight-r1-cpython/Python-3.13.0'); differences=[]
for p in src.rglob('*'):
 if p.is_file():
  q=r1/p.relative_to(src)
  if not q.is_file() or q.read_bytes()!=p.read_bytes():differences.append(str(p.relative_to(src)))
patch=r/'docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/cpython-platform-proposed.patch'
subprocess.run(['patch','-p1','-i',str(patch)],cwd=src,check=True)
(e/'cpython-platform.patch').write_bytes(patch.read_bytes())
assert all((src/p).read_bytes()==(r1/p).read_bytes() for p in ['configure','config.sub'])
py='/tmp/sunlight-r1-cpython/build-host/python'; version=subprocess.check_output([py,'-c','import sys;print(sys.version);assert sys.version_info[:3]==(3,13,0)'],text=True)
(e/'cpython-source.json').write_text(json.dumps(dict(archive_sha256=digest,R1_source_differences_from_pristine=differences,patch='cpython-platform.patch',patched_files_identical_to_R1=True,build_python=py,build_python_version=version),indent=2))
cmd=[str(src/'configure'),'--build='+subprocess.check_output([str(src/'config.guess')],text=True).strip(),'--host=x86_64-unknown-sunlightos','--with-build-python='+py,'--disable-shared','--without-ensurepip','--with-pkg-config=no','--with-libm=']
env=os.environ.copy(); overrides=dict(CC='python3 '+str(r/'tools/native-c/cc.py'),CFLAGS='-O2',LDFLAGS='-static',AR='llvm-ar',RANLIB='llvm-ranlib',CONFIG_SITE='/dev/null',PKG_CONFIG='/bin/false',ac_cv_posix_semaphores_enabled='no')
env.update(overrides)
(e/'configure-environment.json').write_text(json.dumps(dict(command=cmd,environment_overrides=overrides,cache_justifications={'ac_cv_posix_semaphores_enabled':'No semaphore.h or semaphore implementation in native headers/archive; upstream cross default yes is unsupported.'},build='x86_64-pc-linux-gnu',host='x86_64-unknown-sunlightos',target='host program ABI; driver x86_64-unknown-none-elf'),indent=2))
work=b/'cross';work.mkdir();p=subprocess.run(cmd,cwd=work,env=env,capture_output=True,text=True)
(e/'cpython-configure.log').write_text('$ '+shlex.join(cmd)+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n');(e/'cpython-config.log').write_bytes((work/'config.log').read_bytes());(e/'configure-result.json').write_text(json.dumps(dict(exit=p.returncode,build_directory=str(work)),indent=2));print(p.stdout[-3000:]+p.stderr)
