#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <unistd.h>
#include <pthread.h>
#include <fcntl.h>
#include <errno.h>
#include <stddef.h>
struct native_stat {uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,pad;uint32_t nlinks;};
extern int fstat(int,struct native_stat*);
_Static_assert(sizeof(struct native_stat)==24,"native ABI size");
_Static_assert(_Alignof(struct native_stat)==8,"native ABI alignment");
_Static_assert(offsetof(struct native_stat,nlinks)==20,"native ABI offset");
static long raw_stat(const char *p,struct native_stat *s){long r;__asm__ volatile("syscall":"=a"(r):"a"(61L),"D"(p),"S"(s):"rcx","r11","memory");return r;}
static int fd;
static void *writer(void *arg){(void)arg;for(int i=0;i<128;i++)if(write(fd,"abcdefgh",8)!=8)return (void *)1;return 0;}
int main(void){
 struct native_stat s;
 if(raw_stat("/",&s)||s.type!=2)return 1;
 if(raw_stat("/tmp",&s)||s.type!=2)return 2;
 /* Root RAMFS and mounted tmpfs are exercised; no device identity is inferred. */
 if(raw_stat("/etc",&s)||s.type!=2)return 3;
 if(raw_stat("/tmp/py4-missing/file",&s)>=0)return 4;
 if(raw_stat("tmp",&s)>=0||raw_stat("/tmp/../tmp",&s)>=0)return 5;
 char p[]="/tmp/py4-meta-XXXXXX";fd=mkstemp(p);if(fd<0)return 6;
 pthread_t t;if(pthread_create(&t,0,writer,0))return 7;
 for(int i=0;i<2000;i++){if(fstat(fd,&s)||s.type!=1||s.size>1024||s.size%8||(s.mode&0777)!=0600||s.nlinks!=1)return 8;}
 void *r=0;if(pthread_join(t,&r)||r||fstat(fd,&s)||s.size!=1024)return 9;
 if(close(fd)||unlink(p)||raw_stat(p,&s)>=0)return 10;
 puts("PY4 NATIVE METADATA SUBSET PASS root/mount/error/concurrent-size; POSIX stat/lstat/realpath BLOCKED");return 0;
}
