"""Bounded CPython 3.13.0 source-derived header/declaration/static-link probes.
These fragments are NOT a CPython object build or guest execution.
"""
import pathlib,subprocess,json,shlex
root=pathlib.Path.cwd(); out=pathlib.Path(__file__).resolve().parent; dest=out/'api-probes';dest.mkdir(exist_ok=False)
cc=['python3',str(root/'tools/native-c/cc.py'),'-std=c11','-Werror']
def run(name,args):
 p=subprocess.run(list(map(str,args)),capture_output=True,text=True)
 (dest/(name+'.log')).write_text('$ '+shlex.join(list(map(str,args)))+'\n'+p.stdout+p.stderr+'\nexit='+str(p.returncode)+'\n');return p.returncode
headers='assert.h inttypes.h limits.h math.h stdarg.h wchar.h sys/types.h errno.h stdio.h stdlib.h string.h ctype.h unistd.h pthread.h semaphore.h sched.h sys/time.h sys/stat.h fcntl.h signal.h sys/select.h sys/mman.h dlfcn.h locale.h langinfo.h dirent.h poll.h sys/socket.h netdb.h sys/resource.h sys/wait.h termios.h pwd.h grp.h sys/utsname.h'.split()
results=[]
for i,h in enumerate(headers):
 src=dest/('header-'+str(i)+'.c'); src.write_text('#include <'+h+'>\nint main(void){return 0;}\n')
 rc=run('header-'+str(i),cc+['-fsyntax-only',src])
 results.append(dict(requirement=h,kind='header',status='VERIFIED' if rc==0 else 'MISSING' if not (root/'sunlight-libc/include'/h).exists() else 'PARTIAL',header_available=rc==0,evidence='api-probes/header-'+str(i)+'.log',basis='compiler fragment',runtime_verification='NOT_RUN',scope='source-derived candidate; optional headers need configure selection'))
groups={'pthread.h':'pthread_create pthread_join pthread_detach pthread_self pthread_equal pthread_exit pthread_mutex_init pthread_mutex_lock pthread_mutex_unlock pthread_cond_init pthread_cond_destroy pthread_cond_wait pthread_cond_timedwait pthread_cond_signal pthread_cond_broadcast pthread_condattr_init pthread_condattr_setclock pthread_key_create pthread_key_delete pthread_getspecific pthread_setspecific pthread_once pthread_attr_setstacksize pthread_getattr_np', 'stdlib.h':'malloc calloc realloc free getenv setenv unsetenv abort', 'stdio.h':'fopen fclose fread fwrite fseek fflush setvbuf snprintf', 'unistd.h':'read write close lseek getpid isatty fork execv pipe dup', 'time.h':'clock_gettime nanosleep', 'signal.h':'sigaction sigprocmask kill', 'wchar.h':'mbstowcs wcstombs mbrtowc wcrtomb', 'sys/mman.h':'mmap munmap', 'sys/random.h':'getrandom', 'dlfcn.h':'dlopen dlsym'}
nm=subprocess.run(['llvm-nm','-g','--defined-only','target/native-c/sysroot/lib/libsunlight_native_c.a'],capture_output=True,text=True)
(dest/'native-symbols.log').write_text(nm.stdout+nm.stderr)
symbols={l.split()[-1] for l in nm.stdout.splitlines() if len(l.split())>=2}
for h,names in groups.items():
 for name in names.split():
  src=dest/(name+'.c');src.write_text('#include <'+h+'>\nvoid * volatile required_symbol = (void *)&'+name+';\nint main(void) { return required_symbol == 0; }\n')
  decl=run(name+'-compile',cc+['-c',src,'-o',dest/(name+'.o')])
  link=run(name+'-link',cc+[src,'-o',dest/(name+'.elf')]) if decl==0 else None
  results.append(dict(requirement=name,kind='symbol',header=h,header_available=decl==0,link_available=link==0 if link is not None else 'UNKNOWN',archive_export=name in symbols,status='PARTIAL' if decl==0 and link==0 else 'MISSING' if name not in symbols and decl else 'UNKNOWN',evidence=['api-probes/'+name+'-compile.log']+(['api-probes/'+name+'-link.log'] if link is not None else []),basis='compiler/static linker fragment and native archive symbol table',runtime_verification='NOT_RUN',semantic_confidence='unverified by this probe',scope='CPython source candidate; selection conditional'))
(out/'api-probes.json').write_text(json.dumps(results,indent=2))
