"""Sequential image builds; each suite freezes its image before guest dispatch."""
from pathlib import Path
import json, subprocess, sys
from datetime import datetime, timezone
sys.path.insert(0,'tools')
from threading_t2_validate import identity
root=Path('.threading-evidence/t2') / ('validation-run-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
root.mkdir()
print(root, flush=True)
(root/'final-run-source.json').write_text(json.dumps(identity(),indent=2)+'\n')
results=[]
for suite in ['admission','t1','auth','linux','fp','std']:
 command=['python3','tools/threading_t2_regressions.py',suite]
 code=subprocess.run(command).returncode
 results.append({'suite':suite,'command':command,'exit_code':code})
 (root/'final-run-results.json').write_text(json.dumps(results,indent=2)+'\n')
 if code: raise SystemExit(code)
for name,command in [
 ('pin-r2d',['python3','tools/test_remote_wake_smp.py','--pin-r2d','--cpus','1','2','4','12','--timeout','180','--evidence-dir',str(root/'final-pin-r2d')]),
 ('doorbell',['python3','tools/test_native_doorbell.py','--cpus','1','2','4','12','--timeout','180','--evidence-dir',str(root/'final-doorbell')]),
 ('production',['python3','tools/threading_t2_regressions.py','production','--cpus','2'])]:
 code=subprocess.run(command).returncode
 results.append({'suite':name,'command':command,'exit_code':code})
 (root/'final-run-results.json').write_text(json.dumps(results,indent=2)+'\n')
 if code: raise SystemExit(code)
