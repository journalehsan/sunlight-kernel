import pathlib,json,subprocess,hashlib,shutil
E=pathlib.Path(__file__).resolve().parent;R=pathlib.Path.cwd();H=pathlib.Path('/tmp/sunlight-py2-baseline-headers');base=json.loads((E/'source-baseline.json').read_text())['files'];H.mkdir(exist_ok=True)
for n,h in base.items():
 if n.startswith('sunlight-libc/include/'):
  s=(R/n).read_bytes()
  if hashlib.sha256(s).hexdigest()!=h:
   s=s.replace(b'#define EOVERFLOW 75\n',b'').replace(b'int unlink(const char *path);\n',b'').replace(b'char *getenv(const char *name);\nunsigned long strtoul(const char *s, char **end, int base);\n',b'')
   if hashlib.sha256(s).hexdigest()!=h:s=subprocess.check_output(['git','show','HEAD:'+n])
  assert hashlib.sha256(s).hexdigest()==h,n
  p=H/pathlib.Path(n).relative_to('sunlight-libc/include');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(s)
p=json.loads((E/'failure-inventory-before.json').read_text());resource=subprocess.check_output(['clang','-print-resource-dir'],text=True).strip()
for u in p['units']:
 if u['object'] not in ['Modules/getpath.o','Python/sysmodule.o']:continue
 a=u['command'][2:];a[a.index('-o')+1]='/tmp/py2-baseline-'+u['object'].replace('/','_')
 cmd=['clang','--target=x86_64-unknown-none-elf','-march=x86-64','-mno-red-zone','-mno-mmx','-mno-avx','-mno-avx2','-ffreestanding','-fno-pic','-fno-pie','-fno-stack-protector','-nostdinc','-isystem',resource+'/include','-I'+str(H),'--sysroot='+str(R/'target/native-c/sysroot'),*a,'-ferror-limit=0']
 r=subprocess.run(cmd,cwd='/tmp/sunlight-py2-cpython/cross-static',stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);(E/('before-'+u['object'].replace('/','_')+'.log')).write_text(r.stdout);u['diagnostics']=r.stdout;u['diagnostic_command']=cmd;u['baseline_headers_verified']=True;u['first_error']=next((x for x in r.stdout.splitlines() if 'error:' in x),None)
 import re
 u['missing_declarations']=sorted(set(re.findall(r"call to undeclared function '([^']+)'",r.stdout)));u['missing_headers']=re.findall(r"fatal error: '([^']+)' file not found",r.stdout);u['missing_types_or_constants']=sorted(set(re.findall(r"(?:use of undeclared identifier|unknown type name) '([^']+)'",r.stdout)))
(E/'failure-inventory-before.json').write_text(json.dumps(p,indent=2))
