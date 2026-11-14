import pathlib,json,hashlib,difflib
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;base=json.loads((E/'source-baseline.json').read_text());checks={}; patches=[]
def check(n,s):
 patches.extend(difflib.unified_diff(s.splitlines(True),(R/n).read_text().splitlines(True),fromfile='a/'+n,tofile='b/'+n))
 checks[n]={'baseline_sha256':base[n],'reconstructed_sha256':hashlib.sha256(s.encode()).hexdigest(),'matches':hashlib.sha256(s.encode()).hexdigest()==base[n]}
for n,marker in [('sunlight-libc/src/string.rs','\n/// Copy at most n bytes'),('sunlight-libc/src/inttypes.rs','\n// The native ABI has 64-bit long'),('sunlight-libc/src/termination.rs','\n/// Immediate native process-group termination')]:check(n,(R/n).read_text().split(marker)[0])
n='sunlight-libc/src/lib.rs';check(n,(R/n).read_text().replace('\n#[cfg(all(feature = "native-c", target_os = "sunlightos"))]\npub mod condvar;\n',''))
n='sunlight-libc/include/stdlib.h';s=(R/n).read_text();s=s.replace('long strtol(const char *s, char **end, int base);\n','').replace('[[noreturn]] void _Exit(int status);\n','').replace('_Noreturn void _Exit(int status);\n','');check(n,s)
n='sunlight-libc/include/unistd.h';check(n,(R/n).read_text().replace('off_t lseek(int fd, off_t offset, int whence);\n',''))
n='sunlight-libc/include/string.h';s=(R/n).read_text();start=s.index('char *strcpy(');end=s.index('void *memcpy(',start);s=s[:start]+s[end:];check(n,s)
n='sunlight-libc/include/pthread.h';s=(R/n).read_text().replace('#include <bits/time.h>\n','').replace('scheduling/affinity attributes, robust','scheduling/affinity attributes, condition\n * variables, robust');start=s.index('/* Process-private only.');end=s.index('int pthread_create(',start);check(n,s[:start]+s[end:])
(E/'incoming-content-preservation.json').write_text(json.dumps(checks,indent=2));print({n:c['matches'] for n,c in checks.items()})

patches.extend(difflib.unified_diff([], (R/'sunlight-libc/src/condvar.rs').read_text().splitlines(True),fromfile='/dev/null',tofile='b/sunlight-libc/src/condvar.rs'))
(E/'py1-production.patch').write_text(''.join(patches))

n='sunlight-libc/include/time.h';check(n,(R/n).read_text().replace('#include <bits/time.h>','typedef int clockid_t;\nstruct timespec {\n    time_t tv_sec;\n    long tv_nsec;\n};\n#define CLOCK_REALTIME 0\n#define CLOCK_MONOTONIC 1'))
(E/'incoming-content-preservation.json').write_text(json.dumps(checks,indent=2))
patches.extend(difflib.unified_diff([], (R/'sunlight-libc/include/bits/time.h').read_text().splitlines(True),fromfile='/dev/null',tofile='b/sunlight-libc/include/bits/time.h'))
(E/'py1-production.patch').write_text(''.join(patches))
