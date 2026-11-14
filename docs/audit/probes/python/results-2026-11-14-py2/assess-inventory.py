import pathlib,json,re,collections
E=pathlib.Path(__file__).resolve().parent
categories={
'Formatted I/O':['sprintf','snprintf','vsnprintf','vsprintf','printf'],
'File streams':['fdopen','ungetc','BUFSIZ','fopen','FILE','setvbuf','setbuf'],
'Filesystem/stat':['fcntl.h','stat','S_IF','S_IS','open','access','readlink','getcwd','realpath','O_','F_OK','R_OK','W_OK','X_OK','mkdir','rmdir','unlink','rename','chdir'],
'String and memory':['strdup','strerror','strcoll','qsort','strtoul'],
'Wide characters and Unicode support':['wcs','wchar','mbs','mbtow','wctomb','wcrtomb','wmem'],
'Environment':['getenv','setenv','unsetenv','environ','putenv'],
'Time and locale':['time','clock','tm','locale','lconv','tz','strftime','CHAR_MAX'],
'Signals':['sig','SIG'],
'Math':['fabs','copysign','frexp','ldexp','isnan','isinf','HUGE','nan','finite','modf','floor','ceil','sin','cos','exp','atan2','hypot','pow','round','fmod'],
'Platform and ABI':['off_t','pid_t','mode_t'],
'Compiler/toolchain':['unsupported option','unknown target']}
def classify(message):
 words=set(re.findall(r'[A-Za-z_][A-Za-z_0-9]*',message))
 prefixes={'Filesystem/stat':['S_IF','S_IS','O_','st_'],'Wide characters and Unicode support':['wcs','wmem','mbs','wcrtomb','mbtow','wctomb'],'Time and locale':['tm_','clock_','timeval','timespec','locale','tz'],'Signals':['SIG','SA_']}
 for family,names in categories.items():
  if any(n in words or (n.endswith('.h') and n[:-2] in words) for n in names):return family
  if any(word.startswith(prefix) for prefix in prefixes.get(family,[]) for word in words):return family
 if any(w.startswith('sig') and w not in {'signed','signgam','signbit'} for w in words):return 'Signals'
 if words & {'getpid','dup2','_exit'}:return 'Platform and ABI'
 if 'unsupported option' in message or 'unknown target' in message:return 'Compiler/toolchain'
 return 'Other or UNKNOWN'
exports=set(re.findall(r'^\S+ [A-Za-z] (\S+)$',(E/'archive-symbols.log').read_text(),re.M))
for name in ['failure-inventory-before.json','cpython-compilation-matrix.json']:
 p=E/name
 if not p.exists():continue
 data=json.loads(p.read_text());groups=collections.defaultdict(list)
 for u in data['units']:
  d=u.get('diagnostics','');errors=[x for x in d.splitlines() if 'error:' in x];cats=set()
  for line in errors:
   # Classify diagnostic message, not the including file path.
   msg=line.split('error:',1)[-1]
   cat=classify(msg);cats.add(cat)
  u['incomplete_types']=sorted(set(re.findall(r"(?:incomplete definition of type|variable has incomplete type|invalid application of 'sizeof' to an incomplete type) '([^']+)'",d)));u['root_cause_categories']=sorted(cats);u['missing_exported_symbols']=[s for s in u.get('missing_declarations',[]) if s not in exports];u['archive_exports_despite_missing_declaration']=[s for s in u.get('missing_declarations',[]) if s in exports];u['missing_exports_evidence']='Defined-symbol scan of final native archive; not an interpreter link';u['link_evidence']='NOT_RUN'
  for cat in cats:groups[cat].append(u['object'])
  if 'source' in u and u['source']:u['mandatory']=not u['source'].endswith('/unicodedata.c')
 data['root_cause_groups']={k:{'blocked_units':len(groups[k]),'units':groups[k]} for k in list(categories)+['Other or UNKNOWN']};data['configured_optional_units']=['Modules/unicodedata.o'];data['optional_note']='Selected unicodedata remains in the denominator; all other configured units are treated as mandatory for this unchanged minimal static configuration.'
 
 if name=='failure-inventory-before.json':
  for u in data['units']:
   u['missing_exported_symbols']=None;u['missing_exports_evidence']='NOT_RUN: no pre-edit archive scan captured; compiler diagnostics do not prove missing exports';u.pop('archive_exports_despite_missing_declaration',None)
 p.write_text(json.dumps(data,indent=2))
