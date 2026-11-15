import importlib.util,datetime,json,shutil,hashlib,sys
from pathlib import Path
R=Path('/home/ehsantor/Projects/sunlightos-kernel');O=R/'docs/audit/sunfs/sfs-5b4'
for name in ['sfs5b1','sfs5b2','sfs5b3']:
 # name sfs5b1 -> filename sunfs5b1_validate.py
 spec=importlib.util.spec_from_file_location(name,R/f'tools/sunfs{name[3:]}_validate.py')
 mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
 mod.OUT=O
 if hasattr(mod,'base'):mod.base.OUT=O
 if name=='sfs5b3':
  # Original emulator and original patch remain unchanged. The namespace
  # receives a byte-for-byte reference copy needed by the existing runner.
  dest=O/'qemu-active-io.patch'
  if not dest.exists():shutil.copyfile(R/'docs/audit/sunfs/sfs-5b3/qemu-active-io.patch',dest)
 stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
 raw=O/'raw'/f'regression-native-{name}-{stamp}';raw.mkdir()
 source=mod.source if name=='sfs5b1' else mod.base.source
 before=source(); actual_source=source
 if name=='sfs5b1': mod.source=lambda: before
 else: mod.base.source=lambda: before
 (raw/'source-before.json').write_text(json.dumps(before,indent=2)+'\n')
 (raw/'runner.json').write_text(json.dumps(dict(helper_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),reused_runner=f'tools/sunfs{name[3:]}_validate.py',runner_sha256=hashlib.sha256((R/f'tools/sunfs{name[3:]}_validate.py').read_bytes()).hexdigest(),output_redirected=str(O),source_inventory='cached per invocation from frozen manifest, independently verified after complete matrix'),indent=2)+'\n')
 mod.native(raw)
 after=actual_source();(raw/'source-after.json').write_text(json.dumps(after,indent=2)+'\n')
 changed=[n for n in set(before)|set(after) if before.get(n)!=after.get(n)]
 (raw/'input-integrity.json').write_text(json.dumps(dict(status='PASS' if not changed else 'FAIL',changed=changed),indent=2)+'\n')
 print(name,raw,flush=True)
