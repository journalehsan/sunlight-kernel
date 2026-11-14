import pathlib,re,json,csv,hashlib,subprocess
root=pathlib.Path.cwd();p=pathlib.Path(__file__).resolve().parent; src=pathlib.Path('/tmp/sunlight-r1-cpython/Python-3.13.0')
log=(p/'cpython-configure-platform-config.log').read_text()
headers={};symbols={};libraries={}
for i,line in enumerate(log.splitlines(),1):
 for h in re.findall(r"fatal error: '([^']+)' file not found",line): headers.setdefault(h,[]).append(i)
 for s in re.findall(r'undefined symbol: ([^\s]+)',line): symbols.setdefault(s,[]).append(i)
 for lib in re.findall(r'unable to find library (-l[^\s]+)',line): libraries.setdefault(lib,[]).append(i)
observed={'scope':'actual CPython cross-configure diagnostics; feature tests include optional and platform-specific candidates, not a minimal-interpreter requirement list','headers':headers,'undefined_symbols':symbols,'libraries':libraries,'intentional_negative_controls':['ac_nonexistent.h'],'non_requirements':['main (configure compiler identification check)']}
(p/'configure-observed-gaps.json').write_text(json.dumps(observed,indent=2))
(p/'configure-observed-gaps.txt').write_text('Observed missing headers (includes optional/platform checks; exclude ac_nonexistent.h):\n'+'\n'.join(sorted(set(headers)-{'ac_nonexistent.h'}))+'\n\nObserved undefined symbols (exclude compiler-identification main):\n'+'\n'.join(sorted(set(symbols)-{'main'}))+'\n\nObserved unavailable libraries:\n'+'\n'.join(libraries)+'\n')
# Preserve source references for requirements, with no dependence on host config macros.
files=[f for d in ['Include','Python','Objects','Modules'] for f in (src/d).rglob('*') if f.suffix in ['.h','.c']]
texts={str(f.relative_to(src)):f.read_text(errors='replace') for f in files}
def refs(term):
 out=[];pat=re.compile(r'(?<![a-zA-Z_0-9])'+re.escape(term)+r'(?![a-zA-Z_0-9])')
 for name,t in texts.items():
  for i,line in enumerate(t.splitlines(),1):
   if pat.search(line):out.append(name+':'+str(i));break
 return out[:8]
impltexts={str(f):f.read_text(errors='replace') for d in ['sunlight-libc/src','sunlight-libc/include','kernel/src/arch/x86_64'] for f in (root/d).rglob('*') if f.suffix in ['.rs','.h']}
def implementations(term):return [n for n,t in impltexts.items() if re.search(r'\b'+re.escape(term)+r'\b',t)][:8]
def category(term):
 if term.startswith(('pthread','sem_')) or term in ['sched.h']:return 'Threads and synchronization'
 if term.startswith(('clock','time','nanosleep','tz')):return 'Time'
 if term.startswith(('sig','kill')):return 'Signals'
 if term.startswith(('dlopen','dlsym','dlclose')):return 'Dynamic loading'
 if term in ['malloc','free','calloc','realloc','mmap','munmap']:return 'Memory management'
 if term.startswith(('stat','fstat','lstat','open','close','read','write','fseek','ftell','lseek','fopen','fclose','fread','fwrite','dirent','fcntl','pwd','grp','getcwd')):return 'Filesystem'
 return 'libc / POSIX'
items=[]
for row in json.loads((p/'api-probes.json').read_text()):
 term=row['requirement'];r=refs(term)
 items.append({'requirement':term,'kind':row['kind'],'cpython_subsystem':r[0].split('/')[0] if r else 'UNKNOWN','cpython_source_references':r,'category':category(term),'sunlight_implementation_locations':implementations(term),'header_availability':row['header_available'],'link_availability':row.get('link_available','NOT_APPLICABLE'),'runtime_verification':'NOT_RUN for standalone fragment; see native suite for bounded related behavior','semantic_confidence':'declaration/header only' if row['kind']=='header' else 'UNKNOWN beyond static address resolution','classification':row['status'],'blocking_severity':'critical public C API header' if term in ['assert.h','inttypes.h'] else 'thread backend blocker' if term.startswith(('pthread_cond','pthread_key','pthread_getspecific','pthread_setspecific')) else 'conditional/UNKNOWN until final configure selects subsystem','proposed_next_action':'implement only selected requirement and native semantic test' if row['status']=='MISSING' else 'validate CPython-selected semantics','evidence':row['evidence'],'basis':row['basis']})
for kind,table in [('header',headers),('symbol',symbols)]:
 for term,lines in table.items():
  if term in ['ac_nonexistent.h','main']:continue
  items.append({'requirement':term,'kind':kind,'cpython_subsystem':'configure feature detection','cpython_source_references':refs(term),'category':category(term),'sunlight_implementation_locations':implementations(term),'header_availability':'MISSING' if kind=='header' else 'UNKNOWN','link_availability':'failed configure link' if kind=='symbol' else 'UNKNOWN','runtime_verification':'NOT_RUN','semantic_confidence':'unavailable in recorded configure translation/link unit; optionality not established','classification':'MISSING','blocking_severity':'conditional/UNKNOWN; includes optional extensions and other OS probes','proposed_next_action':'select minimal platform configuration before deciding whether to implement','basis':'compiler-observed' if kind=='header' else 'linker-observed','evidence':[f'cpython-configure-platform-config.log:{n}' for n in lines]})
for row in json.loads((p/'math-probes.json').read_text()):
 items.append({'requirement':row['symbol'],'kind':'symbol','cpython_subsystem':'C99 math / configure','cpython_source_references':refs(row['symbol']),'category':'libc / POSIX','sunlight_implementation_locations':implementations(row['symbol']),'header_availability':'UNKNOWN (address probe uses synthetic declaration)','link_availability':row['link_exit']==0,'runtime_verification':'NOT_RUN','semantic_confidence':'address link only; no signature/accuracy assertion','classification':'PARTIAL' if row['link_exit']==0 else 'MISSING','blocking_severity':'configure math gate; minimal required set to review','proposed_next_action':'supply selected C99 math symbols and validate numerics' if row['link_exit'] else 'validate selected numeric semantics','basis':'standalone native linker probe without -lm','evidence':row['evidence']})
# Explicit behavior entries preserve the difference between linked stubs and real semantics.
for term,cl,loc,detail in [('sigprocmask','STUB','sunlight-libc/src/signal.rs:89','returns ENOSYS'),('kill','STUB','sunlight-libc/src/signal.rs:85','returns ENOSYS'),('pthread condition variables','MISSING','sunlight-libc/include/pthread.h','public declarations and exports absent; native futex is not public pthread condvar'),('Rust std condition variables','PARTIAL','tools/native-std/prepare.py','native futex PAL selected for Rust std; not a C pthread export'),('pthread key destructors','MISSING','sunlight-libc/include/pthread.h','no public key API; Rust/C++ TLS cleanup does not establish POSIX key destructors'),('bootstrap imports/encodings','UNKNOWN','kernel/src/process/spawn.rs','no native interpreter build or execution'),('dynamic shared extensions','MISSING','sunlight-libc/src/dlfcn.rs','only fixed main-image namespace supported')]:
 items.append({'requirement':term,'kind':'behavior','cpython_subsystem':'platform integration','category':category(term),'sunlight_implementation_locations':[loc],'header_availability':'UNKNOWN','link_availability':'UNKNOWN','runtime_verification':'NOT_RUN in CPython','semantic_confidence':detail,'classification':cl,'blocking_severity':'optional extensions' if term=='dynamic shared extensions' else 'review selected bootstrap backend','proposed_next_action':'R2 platform contract and native race/bootstrap tests','basis':'source-analysis','evidence':[loc]})
(p/'cpython-compatibility-inventory.json').write_text(json.dumps({'reference':'CPython 3.13.0','scope':'bounded discovered requirements; no completeness percentage; header VERIFIED means compiled header only, never runtime semantics','requirements':items},indent=2))
print('Inventory rows',len(items),'observed headers',len(headers)-1,'observed undefined',len(symbols)-1)
