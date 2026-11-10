"""Generate independent configure-style C API probes, only beside this file."""
from pathlib import Path
p=Path(__file__).resolve().parent
probes={
'stdio':('#include <stdio.h>','FILE *f=tmpfile(); if(!f) return 1; if(fputs("x",f)<0) return 2; rewind(f); int x=fgetc(f); fclose(f); return x!=\'x\';'),
'string':('#include <string.h>','char b[8]; memcpy(b,"abc",4); return strcmp(b,"abc");'),
'locale':('#include <locale.h>','return setlocale(LC_ALL,"C")==0;'),
'math':('#include <math.h>','volatile double x=2.0; return fabs(sqrt(x)*sqrt(x)-x)>1e-12;'),
'clock':('#include <time.h>','struct timespec a,b,d={0,1000000}; return clock_gettime(CLOCK_REALTIME,&a)||clock_gettime(CLOCK_MONOTONIC,&b)||nanosleep(&d,0);'),
'dirent':('#include <dirent.h>','DIR *d=opendir("."); if(!d) return 1; int bad=readdir(d)==0; closedir(d); return bad;'),
'poll_select':('#include <poll.h>\n#include <sys/select.h>','struct timeval t={0,0}; return poll(0,0,0)!=0 || select(0,0,0,0,&t)!=0;'),
'socket':('#include <sys/socket.h>\n#include <unistd.h>','int f=socket(AF_INET,SOCK_STREAM,0); if(f<0)return 1; return close(f);'),
'pthread':('#include <pthread.h>\nstatic _Thread_local int tls=7; static void *worker(void *p){(void)p; return tls==7 ? 0 : (void*)1;}','pthread_t t; void *v; if(pthread_create(&t,0,worker,0))return 1; return pthread_join(t,&v)||v!=0;'),
'dlfcn':('#include <dlfcn.h>','void *h=dlopen(0,RTLD_NOW); if(!h)return 1; int bad=dlsym(h,"malloc")==0; dlclose(h); return bad;'),
'signal':('#include <signal.h>\nstatic volatile sig_atomic_t seen; static void h(int s){seen=s;}','struct sigaction a={0}; a.sa_handler=h; sigemptyset(&a.sa_mask); if(sigaction(SIGUSR1,&a,0)||raise(SIGUSR1))return 1; return seen!=SIGUSR1;'),
'wchar':('#include <wchar.h>\n#include <locale.h>','setlocale(LC_ALL,"C"); wchar_t w=0; return mbrtowc(&w,"A",1,0)!=1||w!=L\'A\';'),
'langinfo':('#include <langinfo.h>','return nl_langinfo(CODESET)==0;'),
'atexit':('#include <stdlib.h>\n#include <unistd.h>\nstatic void bye(void){write(1,"atexit OK\\n",10);}','return atexit(bye);'),
'mmap':('#include <sys/mman.h>','void *p=mmap(0,1048576,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0); if(p==MAP_FAILED)return 1; ((char*)p)[1048575]=1; return mprotect(p,1048576,PROT_READ)||munmap(p,1048576);'),
'getrandom':('#include <sys/random.h>','char b[16]; return getrandom(b,sizeof b,0)!=sizeof b;'),
# PTY-independent negative-path check; interactive behavior requires the tty probe.
 'termios':('#include <termios.h>\n#include <errno.h>\n#include <unistd.h>','struct termios t; int r=tcgetattr(0,&t); return r==0 || errno==ENOTTY ? 0 : 1;')}
for name,(inc,body) in probes.items():
    (p/(name+'.c')).write_text('#define _GNU_SOURCE\n'+inc+'\nint main(void){'+body+'}\n')
