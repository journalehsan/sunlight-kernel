#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#define CHECK(x) do { if (!(x)) { printf("PY7 FAIL line=%d errno=%d\n",__LINE__,errno);return __LINE__; } } while(0)
static long call(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
struct native_stat { uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,reserved;uint32_t links; };
static int contents(int fd,const char *expected,int n) {char b[32];return lseek(fd,0,SEEK_SET)==0&&read(fd,b,n)==n&&!memcmp(b,expected,n);}
int main(void) {
 const char *a="/state/users/1000/A.TXT",*b="/state/users/1000/B.TXT";
 int old=open(a,O_CREAT|O_EXCL|O_RDWR,0600);CHECK(old>=0);CHECK(write(old,"old",3)==3);
 int alias=open("/state/USERS/1000/a.txt",O_RDONLY);CHECK(alias>=0);
 int duplicated=(int)call(45,old,0,0);CHECK(duplicated>=0);
 CHECK(!unlink(a));
 int replacement=open(a,O_CREAT|O_EXCL|O_RDWR,0600);CHECK(replacement>=0);CHECK(write(replacement,"new",3)==3);
 char out[8];struct native_stat st;
 errno=0;CHECK(read(old,out,3)==-1&&errno==EBADF);
 errno=0;CHECK(write(old,"BAD",3)==-1&&errno==EBADF);
 errno=0;CHECK(read(alias,out,3)==-1&&errno==EBADF);
 errno=0;CHECK(write(duplicated,"BAD",3)==-1&&errno==EBADF);
 CHECK(call(48,old,(long)&st,0)==-5);
 CHECK(contents(replacement,"new",3));
 CHECK(!close(old));CHECK(!close(alias));CHECK(!close(duplicated));
 CHECK(!call(66,(long)a,(long)b,0));CHECK(contents(replacement,"new",3));
 CHECK(write(replacement,"!",1)==1);
 int victim=open(a,O_CREAT|O_EXCL|O_RDWR,0600);CHECK(victim>=0);CHECK(write(victim,"victim",6)==6);
 CHECK(!call(66,(long)b,(long)a,0));
 errno=0;CHECK(write(victim,"BAD",3)==-1&&errno==EBADF);
 errno=0;CHECK(read(victim,out,3)==-1&&errno==EBADF);
 CHECK(contents(replacement,"new!",4));
 CHECK(!close(victim));CHECK(!close(replacement));CHECK(!unlink(a));
 puts("PY7 FAT PASS: unlink read/write/dup rejected; replacement bytes preserved; rename source retained; destination revoked; valid closes");
 return 0;
}
