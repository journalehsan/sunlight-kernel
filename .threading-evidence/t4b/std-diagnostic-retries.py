import sys,json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0,str(Path.cwd()/'tools'))
from threading_t4b_validate import run
iso=Path('.threading-evidence/t4b/20261105T190921.925242Z-std-diagnostic/boot.iso')
markers=['[NATIVE-STD] T4A mutex/condvar handoffs=1536 independent=4 progress/TLS/resources OK','[NATIVE-STD] collections/clocks/locks/TLS/join OK','[NATIVE-STD] T2 Hayro workers=4 concurrent render/join OK','[NATIVE-STD] shared descriptors/OOM/group exit OK','[DOCSERV] native Hayro IPC open/render/close OK']
def guest(i):
 ok,path=run('std-diagnostic-retry-'+str(i),[],180,iso,12,markers)
 return {'pass':ok,'attempt':str(path)}
with ThreadPoolExecutor(max_workers=2) as pool:results=list(pool.map(guest,range(2)))
Path('.threading-evidence/t4b/std-diagnostic-retries.json').write_text(json.dumps(results,indent=2)+'\n')
