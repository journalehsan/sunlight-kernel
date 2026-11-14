import os,pathlib,subprocess,json,shlex,hashlib,base64,difflib
root=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent; base=pathlib.Path('/tmp/sunlight-r1-cpython');src=base/'Python-3.13.0';py=base/'build-host/python'
def run(name,cmd,cwd,env=None):
 p=subprocess.run(list(map(str,cmd)),cwd=cwd,env=env,capture_output=True,text=True)
 (e/(name+'.log')).write_text('$ '+shlex.join(list(map(str,cmd)))+'\ncwd='+str(cwd)+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n')
 if (cwd/'config.log').exists(): (e/(name+'-config.log')).write_bytes((cwd/'config.log').read_bytes())
 return p.returncode
run('build-python-version',[py,'-c','import sys; print(sys.version); assert sys.version_info[:3] == (3,13,0)'],base)
archive=pathlib.Path('/tmp/sunlight-r1-Python-3.13.0.tar.xz').read_bytes(); bundle=json.loads((e/'cpython-archive.sigstore.json').read_text()); digest=hashlib.sha256(archive).digest()
assert digest==base64.b64decode(bundle['messageSignature']['messageDigest']['digest'])
(e/'cpython-origin.json').write_text(json.dumps({'version':'3.13.0','origin':'https://www.python.org/ftp/python/3.13.0/Python-3.13.0.tar.xz','sha256':digest.hex(),'md5':hashlib.md5(archive).hexdigest(),'release_page_md5':'726e5b829fcf352326874c1ae599abaa','integrity':'MD5 matches official release page; SHA256 matches downloaded official Sigstore bundle digest; signature/certificate trust NOT independently verified','build_python':str(py),'host_configure':'../Python-3.13.0/configure --without-ensurepip','host_build':'make -j8 python','host_build_exit':0},indent=2))
build=subprocess.check_output([str(src/'config.guess')],text=True).strip()
cmd=[src/'configure','--build='+build,'--host=x86_64-unknown-sunlightos','--with-build-python='+str(py),'--disable-shared','--without-ensurepip','--with-pkg-config=no']
env=os.environ.copy()
# CC invokes native driver; no host includes or host libraries, no target cache guesses.
env.update(CC='python3 '+str(root/'tools/native-c/cc.py'), CFLAGS='-O2',LDFLAGS='-static',AR='llvm-ar',RANLIB='llvm-ranlib',CONFIG_SITE='/dev/null',PKG_CONFIG='/bin/false')
(e/'configure-environment.json').write_text(json.dumps({'command':list(map(str,cmd)),'build':build,'host':'x86_64-unknown-sunlightos','target':'host (Autoconf program build); LLVM x86_64-unknown-none-elf via native driver','environment_overrides':{k:env[k] for k in ['CC','CFLAGS','LDFLAGS','AR','RANLIB','CONFIG_SITE','PKG_CONFIG']},'cache_assumptions':{},'cross_cache_overrides':{}},indent=2))
work=base/'cross-original';work.mkdir(exist_ok=False);run('cpython-configure-original',cmd,work,env)
# Experimental platform recognition only, in extracted disposable sources.
old=(src/'config.sub').read_text();new=old.replace('| os2* | vos*','| sunlightos* | os2* | vos*');assert old!=new
(src/'config.sub').write_text(new)
patch=''.join(difflib.unified_diff(old.splitlines(True),new.splitlines(True),fromfile='a/config.sub',tofile='b/config.sub'))
work=base/'cross-config-sub';work.mkdir(exist_ok=False);run('cpython-configure-config-sub',cmd,work,env)
old=(src/'configure').read_text();new=old.replace('case "$host" in\n','case "$host" in\n\t*-*-sunlightos*)\n\t\tac_sys_system=SunlightOS\n\t\t_host_ident=$host_cpu\n\t\t;;\n')
assert old!=new
(src/'configure').write_text(new)
patch+=''.join(difflib.unified_diff(old.splitlines(True),new.splitlines(True),fromfile='a/configure',tofile='b/configure'))
(e/'cpython-platform-experimental.patch').write_text(patch)
work=base/'cross-platform';work.mkdir(exist_ok=False);run('cpython-configure-platform',cmd,work,env)
