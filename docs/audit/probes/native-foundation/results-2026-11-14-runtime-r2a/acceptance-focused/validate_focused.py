import pathlib,subprocess,shlex,json,ctypes
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent
results=[]
def run(label,cmd,expected=0):
 p=subprocess.run(list(map(str,cmd)),cwd=r,capture_output=True,text=True)
 (e/(label+'.log')).write_text('$ '+shlex.join(list(map(str,cmd)))+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n')
 results.append(dict(name=label,exit=p.returncode,expected=expected,status='PASS' if p.returncode==expected else 'FAIL',command=list(map(str,cmd))))
 (e/'focused-results.json').write_text(json.dumps(results,indent=2));assert p.returncode==expected,(label,p.returncode)
cc=['python3','tools/native-c/cc.py']; cxx=['python3','tools/native-c/cxx.py']
# All format macros must match actual target types, including FAST16 == short.
src='#include <inttypes.h>\nextern int p(const char *, ...) __attribute__((format(printf,1,2)));\nextern int s(const char *, ...) __attribute__((format(scanf,1,2)));\nvoid check(void) {\n'
for suffix,typ in [(n,'int'+n+'_t') for n in ['8','16','32','64']]+[(k.upper()+n,'int_'+k+n+'_t') for k in ['least','fast'] for n in ['8','16','32','64']]+[('MAX','intmax_t'),('PTR','intptr_t')]:
 for spec in 'diouxX':
  t=typ if spec in 'di' else 'u'+typ
  src+=f'p("%" PRI{spec}{suffix}, ({t})0);\n'
 for spec in 'dioux':
  t=typ if spec in 'di' else 'u'+typ
  src+=f's("%" SCN{spec}{suffix}, ({t} *)0);\n'
src+='}\n';(e/'format-types.c').write_text(src)
run('format-types-c',cc+['-std=c11','-Wall','-Wextra','-Werror','-fsyntax-only',e/'format-types.c'])
run('format-types-cxx',cxx+['-x','c++','-std=c++17','-Wall','-Wextra','-Werror','-fsyntax-only',e/'format-types.c'])
# Independent host byte-class oracle, exhaustive valid ctype argument domain.
lib=ctypes.CDLL(None); names=['isalnum','isalpha','isblank','iscntrl','isdigit','isgraph','islower','isprint','ispunct','isspace','isupper','isxdigit']
body='#include <ctype.h>\n#include <stdio.h>\nint main(void) {\n'
for c in range(-1,256):
 for name in names:
  val=bool(getattr(lib,name)(c));body+=f'if (!!{name}({c}) != {int(val)}) return 1;\n'
 for name in ['toupper','tolower']:body+=f'if ({name}({c}) != {getattr(lib,name)(c)}) return 2;\n'
body+='puts("R2A ctype PASS 257 inputs x 14 functions");return 0;}\n';(e/'ctype-native.c').write_text(body)
for label,source in [('math','math-native.c'),('headers-c','headers-native.c'),('ctype','ctype-native.c')]:
 run('final-'+label+'-build',cc+['-std=c11','-O2','-Wall','-Wextra','-Werror','-fno-builtin',e/source,'-o',e/('final-'+label+'.elf')])
run('final-headers-cxx-object',cxx+['-x','c++','-std=c++17','-O2','-Wall','-Wextra','-Werror','-c',e/'headers-native.c','-o',e/'final-headers-cxx.o'])
run('final-headers-cxx-link',cxx+[e/'final-headers-cxx.o','-o',e/'final-headers-cxx.elf'])
for label,status,args in [('math',0,[]),('headers-c',0,[]),('headers-cxx',0,[]),('ctype',0,[]),('headers-c',134,['--arg','fail']),('headers-cxx',134,['--arg','fail'])]:
 name='final-'+label+('-assert' if status else '')
 run(name+'-runner',['python3','tools/native-c/guest.py',e/('final-'+label+'.elf'),'--cpus','2','--timeout','180','--expect-exit',str(status),*args,'--out',e/(name+'-guest')])
 if status:
  stdout=(e/(name+'-guest')/'elf.stdout.txt').read_text();stderr=(e/(name+'-guest')/'elf.stderr.txt').read_text()
  assert not stdout and 'Assertion' in stderr and 'headers-native.c:' in stderr and 'main:' in stderr
print('Focused header/math/ctype/C++/assert checks PASS')
