/* Exercise the EXISTING 24-byte native metadata ABI; this is not POSIX stat. */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
struct native_stat {uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,pad;uint32_t nlinks;};
extern int fstat(int,struct native_stat*);
_Static_assert(sizeof(struct native_stat)==24,"native ABI");
static long raw_stat(const char *path,struct native_stat *s){long r;__asm__ volatile("syscall":"=a"(r):"a"(61L),"D"(path),"S"(s):"rcx","r11","memory");return r;}
#define CHECK(x) do {if(!(x)){printf("FS FAIL line=%d errno=%d\n",__LINE__,errno);return __LINE__;}}while(0)
int main(void){char path[]="/tmp/py3-meta-XXXXXX";int fd=mkstemp(path);CHECK(fd>=0);struct native_stat a,b;CHECK(!fstat(fd,&a));CHECK(a.size==0&&a.type==1&&(a.mode&0777)==0600&&a.nlinks==1);CHECK(write(fd,"abc",3)==3);CHECK(!fstat(fd,&a)&&a.size==3);CHECK(!raw_stat(path,&b)&&a.size==b.size&&a.uid==b.uid&&a.gid==b.gid&&a.mode==b.mode&&a.type==b.type&&a.nlinks==b.nlinks);CHECK(!raw_stat("/tmp",&b)&&b.type==2);CHECK(raw_stat("/tmp/py3-absent-component/file",&b)<0);CHECK(raw_stat("tmp",&b)<0);CHECK(raw_stat("/tmp/../tmp",&b)<0);CHECK(raw_stat("/tmp/.",&b)<0);errno=0;CHECK(fstat(-1,&b)==-1&&errno==EBADF);errno=0;CHECK(fstat(fd,0)==-1&&errno==EFAULT);CHECK(!close(fd));errno=0;CHECK(fstat(fd,&b)==-1&&errno==EBADF);CHECK(!unlink(path));CHECK(raw_stat(path,&b)<0);puts("PY3 NATIVE METADATA SUBSET PASS; POSIX stat/realpath BLOCKED");return 0;}
