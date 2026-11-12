import pathlib,json,hashlib,re
E=pathlib.Path(__file__).resolve().parent
root=E.parents[4]
import os
os.chdir(root)
p=json.loads((E/'incoming.json').read_text())
for name,h in p.items():
 data=pathlib.Path(name).read_bytes();s=None
 if name=='sunlight-cxx/README.md':
  s=data.decode().split('\n## Owning strings (Phase 21)')[0]
  s=s.replace('# Bounded native C++ output and string library','# Bounded native C++ output library').replace('This is a compiled output and owning byte-string subset','This is a compiled, allocation-free output subset').replace('formatting, standard exception','formatting, owning strings, standard exception').replace('now fails at `<stdexcept>`;', 'now fails at `<string>`;').replace('hash and next missing header (`<stdexcept>` after Phase 21).','hash and next missing header.')
 elif name=='sunlight-cxx/include/iostream':
  s=data.decode();s=re.sub(r'#if __cplusplus >= 201703L\n.*?\n#endif\n','',s,flags=re.S).replace(' noexcept','')
 elif name=='sunlight-cxx/src/ostream.cpp':
  s=data.decode().replace('using size_t = __SIZE_TYPE__;\nusing ssize_t = long;\nextern "C" ssize_t write(int, const void*, size_t) noexcept;', '#include <unistd.h>').replace(' noexcept','')
 elif name=='tools/build_native_cxx.sh':
  s=data.decode();s=''.join(x for x in s.splitlines(keepends=True) if ' -c sunlight-cxx/src/string.cpp ' not in x).replace(' target/native-cxx/string.o','')
 elif name=='tools/native-c/tests/run_cxx_output.py':
  s=data.decode().replace('stdexcept','string')
 elif name=='tools/native-c/guest.py':
  s=data.decode();a=s.index("        sources += [('cxx-string',");b=s.index("        sources += [('wchar-contract'",a);s=s[:a]+s[b:]
  s=s.replace("if name in ('cxx-backend', 'cxx-string-backend'):","if name == 'cxx-backend':")
  a=s.index("                if name == 'cxx-string':");b=s.index("            if command('build-' + name",a);s=s[:a]+s[b:]
 elif name=='docs/audit/std-progress.md':
  data=data.split(b'\n## Phase 21:')[0]
 if s is not None:data=s.encode()
 assert hashlib.sha256(data).hexdigest()==h,'incoming prefix/content lost: '+name
for name,h in json.loads((E/'historical.json').read_text()).items():
 assert hashlib.sha256(pathlib.Path(name).read_bytes()).hexdigest()==h,name
print('Incoming work preserved:',len(p),'files; all historical root probes unchanged')
