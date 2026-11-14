#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <limits.h>
#include <fcntl.h>
#define CHECK(x) do { if (!(x)) { printf("PY6 FAIL line=%d errno=%d\n", __LINE__, errno); return __LINE__; } } while(0)
static long call(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
_Static_assert(PATH_MAX==256,"native path capacity");
int main(void){
 char output[PATH_MAX], original[PATH_MAX];
 /* Capability-denied prefix must not be erased by a later '..'. */
 CHECK(call(179,(long)"/etc/sunlight/py6-protected/../../tmp",(long)output,sizeof output)==-4);
 memset(output,0x5a,sizeof output);memcpy(original,output,sizeof output);
 CHECK(call(179,(long)"/tmp",(long)output,4)==-10);
 CHECK(!memcmp(output,original,sizeof output));
 CHECK(call(179,(long)"/tmp",(long)output,sizeof output)==4&&!strcmp(output,"/tmp"));
 CHECK(call(179,(long)"/tmp/py6-missing/../",(long)output,sizeof output)==-3);
 errno=123;CHECK(realpath("/tmp//./../tmp",output)==output&&!strcmp(output,"/tmp")&&errno==123);
 char *allocated=realpath("/tmp",NULL);CHECK(allocated&&!strcmp(allocated,"/tmp"));free(allocated);
 errno=0;CHECK(!realpath("/tmp/py6-missing/../",output)&&errno==ENOENT);
 errno=0;CHECK(!realpath("",output)&&errno==ENOENT);
 errno=0;CHECK(!realpath(NULL,output)&&errno==EINVAL);
 char too_long[PATH_MAX+1];memset(too_long,'x',PATH_MAX);too_long[PATH_MAX]=0;
 errno=0;CHECK(!realpath(too_long,output)&&errno==ENAMETOOLONG);
 CHECK(!call(63,(long)"/tmp//./",0,0));
 CHECK(realpath(".",output)&&!strcmp(output,"/tmp"));
 CHECK(realpath("../tmp",output)&&!strcmp(output,"/tmp"));
 int fd=open("/tmp/ py6 spaced ",O_CREAT|O_EXCL|O_RDWR,0600);CHECK(fd>=0);
 CHECK(write(fd,"payload",7)==7);
 CHECK(realpath("./ py6 spaced ",output)&&!strcmp(output,"/tmp/ py6 spaced "));
 errno=0;CHECK(!realpath("/tmp/ py6 spaced /..",output)&&errno==ENOTDIR);
 errno=0;CHECK(!realpath("/tmp/ py6 spaced /",output)&&errno==ENOTDIR);
 CHECK(!close(fd));CHECK(!unlink("/tmp/ py6 spaced "));
 CHECK(!call(63,(long)"/",0,0));
 puts("PY6 canonical PASS: component existence, cwd, mount, whitespace, allocation, bounds, errno; symlinks NOT_RUN");
 return 0;
}
