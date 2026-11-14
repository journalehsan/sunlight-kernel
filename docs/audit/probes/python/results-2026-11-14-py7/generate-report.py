from pathlib import Path
import json
E=Path(__file__).resolve().parent
read=lambda n:json.loads((E/n).read_text())
x=read('validation-summary.json');m=read('cpu-matrix.json');w=read('fat-stale-descriptor-witnesses.json');integrity=read('source-integrity.json');production=read('modified-production-files.json')
s=(E/'report-template.md').read_text()
s=s.replace('{{gates}}','| Gate | Status | Evidence boundary |\n|---|---|---|\n'+'\n'.join('| '+k+' | '+v['status']+' | '+v['reason']+' |' for k,v in x['gates'].items()))
s=s.replace('{{baseline_count}}',f"{integrity['baseline_regular_files']:,}")
s=s.replace('{{witnesses}}','| Witness | Status | Qualification |\n|---|---|---|\n'+'\n'.join('| '+k+' | '+v['status']+' | '+v['scope']+' |' for k,v in w['witnesses'].items()))
c=x['compilation'];s=s.replace('{{cpython}}',f"**Configured {c['configured']}; attempted {c['attempted']}; compiled {c['passed']}; failed {c['failed']}; unattempted {c['unattempted']}.** The configured object set is unchanged. Newly compiled objects: {len(c['newly_compiled'])}. No improvement over the PY-6 165/180 count is claimed.")
def summary(c):return str(c.get('passed',0))+'/'+str(c.get('expected','?'))+' '+c['status']
s=s.replace('{{cpus}}','| CPUs | C | C++ | Lifecycle | Existing functional | PY6 | PY7 FAT | Directory subset | TCP |\n|---|---|---|---|---|---|---|---|---|\n'+'\n'.join('| '+cpu+' | '+' | '.join(summary(c[k]) for k in ['C','CXX','lifecycle','existing_functional','PY6','PY7_FAT','PY7_directory_subset'])+' | '+c['TCP']['status']+' |' for cpu,c in m['cpus'].items()))
s=s.replace('{{files}}','\n'.join('- `'+p['path']+'`' for p in production))
assert '{{' not in s
Path('docs/runtime/python-py7-filesystem-lifetime-posix-links.md').write_text(s)
print(x['overall'],x['regressions'],integrity['status'])
