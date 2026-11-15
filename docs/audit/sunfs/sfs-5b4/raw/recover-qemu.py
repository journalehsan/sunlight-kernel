from pathlib import Path
import hashlib,tarfile,subprocess,json,shutil
R=Path('/home/ehsantor/Projects/sunlightos-kernel');O=R/'docs/audit/sunfs/sfs-5b4';A=Path('/tmp/sfs5b3-qemu-11.1.2.tar.xz');S=Path('/tmp/sfs5b4-qemu-src');B=Path('/tmp/sfs5b4-qemu-build')
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert sha(A)=='731b5681e4bb18be313231579b8efd0296c5b015fa36dc533874b639ba838016'
S.mkdir();B.mkdir()
with tarfile.open(A) as t:
 for member in t.getmembers():
  member.name=member.name.split('/',1)[-1]
  if member.name and not (member.issym() and member.linkname.startswith('/')):t.extract(member,S,filter='data')
subprocess.run(['patch','-p1','-i',str(O/'qemu-async-access.patch')],cwd=S,check=True)
argv=[str(S/'configure'),'--target-list=x86_64-softmmu','--disable-docs','--disable-werror','--disable-download','--disable-gtk','--disable-sdl','--disable-opengl','--disable-virglrenderer','--disable-vnc','--disable-tools','--disable-user','--disable-plugins']
with (O/'raw/qemu-recovery-configure.log').open('x') as f:subprocess.run(argv,cwd=B,stdout=f,stderr=subprocess.STDOUT,check=True,timeout=300)
with (O/'raw/qemu-recovery-build.log').open('x') as f:subprocess.run(['ninja','-j','1','qemu-system-x86_64'],cwd=B,stdout=f,stderr=subprocess.STDOUT,check=True,timeout=3600)
p=O/'qemu-source-integrity.json';old=json.loads(p.read_text());(O/'raw/qemu-identity-before-cargo-clean.json').write_text(json.dumps(old,indent=2)+'\n')
manifest={str(p.relative_to(S)):sha(p) for p in S.rglob('*') if p.is_file() and not p.is_symlink()};m=O/'raw/qemu-source-manifest-recovery.json';m.write_text(json.dumps(manifest,indent=2)+'\n')
old.update(binary=str(B/'qemu-system-x86_64'),binary_sha256=sha(B/'qemu-system-x86_64'),source_manifest='raw/qemu-source-manifest-recovery.json',source_manifest_sha256=sha(m),configure_log='raw/qemu-recovery-configure.log',build_log='raw/qemu-recovery-build.log',build_command=['ninja','-j','1','qemu-system-x86_64'],recovery='Restored pinned archive and unchanged v2 patch after cargo clean; previous binary identity retained')
p.write_text(json.dumps(old,indent=2)+'\n');print('RECOVERED QEMU',old['binary_sha256'],flush=True)
