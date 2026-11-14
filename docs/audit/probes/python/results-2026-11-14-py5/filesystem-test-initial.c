#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
struct native_stat {uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,pad;uint32_t nlinks;};
extern int fstat(int,struct native_stat*);
_Static_assert(sizeof(struct native_stat)==24,"native subset ABI");
_Static_assert(offsetof(struct native_stat,nlinks)==20,"native subset ABI");
static long call3(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
static long meta(const char*p,struct native_stat*s){return call3(61,(long)p,(long)s,0);}
int main(void){
 struct native_stat s; char path[]="/tmp/py5-fd-XXXXXX";
 int fd=mkstemp(path);if(fd<0||write(fd,"data",4)!=4)return 1;
 if(meta(path,&s)||s.type!=1||s.size!=4||(s.mode&0777)!=0600)return 2;
 if(rename(path,"/tmp/py5-renamed")||meta(path,&s)>=0||fstat(fd,&s)||s.size!=4)return 3;
 if(unlink("/tmp/py5-renamed")||fstat(fd,&s)||s.size!=4||s.nlinks!=0)return 4;
 if(close(fd)||fstat(fd,&s)!=-1||errno!=EBADF)return 5;
 if(meta("/",&s)||s.type!=2||meta("/tmp",&s)||s.type!=2)return 6;
 if(chdir("/tmp"))return 7;
 /* Record existing divergence: open resolves CWD, native StatPath61 does not. */
 fd=open("py5-relative",O_CREAT|O_EXCL|O_RDWR,0600);if(fd<0)return 8;
 if(meta("py5-relative",&s)>=0||meta("/tmp/py5-relative",&s)||meta("/tmp/./py5-relative",&s)>=0)return 9;
 if(meta("/tmp//py5-relative",&s)>=0||meta("",&s)>=0||meta("/tmp/py5-relative/",&s)>=0)return 10;
 /* Exercise mounted handle_path through existing getdents64 compatibility adapter.
    This does not claim Linux process ABI, POSIX stat or canonical resolution. */
 int dir=open("/tmp",O_RDONLY);if(dir<0)return 11;
 unsigned char buf[2048];long n=call3(1034,dir,(long)buf,sizeof(buf));if(n<=0)return 12;
 int found=0;for(long i=0;i<n;){if(n-i<20)return 13;unsigned short len;memcpy(&len,buf+i+16,2);if(len<20||len>n-i)return 14;
 if(!memchr(buf+i+19,0,len-19))return 15;if(!strcmp((char*)buf+i+19,"py5-relative"))found=1;i+=len;}
 if(!found||close(dir)||close(fd)||unlink("/tmp/py5-relative")||chdir("/"))return 16;
 puts("PY5 native subset PASS: rename/unlink descriptor identity, mount getdents, cwd divergence, invalid paths; POSIX metadata/realpath BLOCKED");return 0;
}
