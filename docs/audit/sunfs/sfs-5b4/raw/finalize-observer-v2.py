from pathlib import Path
import hashlib,json,shutil,subprocess,time
R=Path('/home/ehsantor/Projects/sunlightos-kernel');O=R/'docs/audit/sunfs/sfs-5b4';S=Path('/tmp/sfs5b4-qemu-src')
end=time.monotonic()+300
ready=O/'raw/native-20261115T083525.831072Z/input-integrity.json'
while not ready.exists():
 if time.monotonic()>=end:raise SystemExit('bounded wait expired; old sweep not finished; no replacement')
 time.sleep(1)
shutil.copyfile('/tmp/sfs5b4-observer-v2.patch',O/'qemu-async-access.patch')
with (O/'raw/qemu-build-4.log').open('x') as log:
 result=subprocess.run(['ninja','-j','6','qemu-system-x86_64'],cwd='/tmp/sfs5b4-qemu-build',stdout=log,stderr=subprocess.STDOUT,timeout=300)
assert result.returncode==0
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
for manifest in ['qemu-source-manifest.json','qemu-reviewed-source.json']:
 p=O/'raw'/manifest;v=json.loads(p.read_text());v['hw/block/virtio-blk.c']=sha(S/'hw/block/virtio-blk.c');p.write_text(json.dumps(v,indent=2)+'\n')
shutil.copyfile(S/'hw/block/virtio-blk.c',O/'raw/qemu-reviewed-source/hw/block/virtio-blk.c')
p=O/'qemu-source-integrity.json';v=json.loads(p.read_text());v.update(patch_sha256=sha(O/'qemu-async-access.patch'),binary_sha256=sha(v['binary']),source_manifest_sha256=sha(O/'raw/qemu-source-manifest.json'),previous_build_log=v['build_log'],build_log='raw/qemu-build-4.log',observer_revision='v2: recorded positive control before acknowledgment; raw/observer-gap-v1 preserves rejected model')
p.write_text(json.dumps(v,indent=2)+'\n')
print('QEMU v2 ready',v['binary_sha256'],flush=True)
