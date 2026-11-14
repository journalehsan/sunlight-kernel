import pathlib,subprocess,json
E=pathlib.Path(__file__).resolve().parent;resource=subprocess.check_output(['clang','-print-resource-dir'],text=True).strip();rows=[]
for lang,std in [('c','c11'),('c++','c++17')]:
 for reverse in [False,True]:
  headers=['time.h','pthread.h'] if reverse else ['pthread.h','time.h'];src=''.join('#include <'+h+'>\n' for h in headers)+'\nint main(void){return 0;}\n'
  cmd=['clang','--target=x86_64-unknown-none-elf','-ffreestanding','-nostdinc','-isystem',resource+'/include','-I','sunlight-libc/include','-std='+std,'-pedantic-errors','-Wall','-Wextra','-Werror','-x',lang,'-fsyntax-only','-']
  p=subprocess.run(cmd,input=src,capture_output=True,text=True);rows.append(dict(language=lang,reverse=reverse,exit=p.returncode,source=src,command=cmd,diagnostics=p.stdout+p.stderr))
(E/'clock-header-order-verified.json').write_text(json.dumps(rows,indent=2));assert all(x['exit']==0 for x in rows)
print('clock/pthread include order C11/C++17 PASS')
