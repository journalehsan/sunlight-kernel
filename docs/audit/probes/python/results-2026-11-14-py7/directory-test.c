#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <unistd.h>
#include <fcntl.h>
#define CHECK(x) do {if(!(x)){printf("PY7 directory FAIL %d\n",__LINE__);return __LINE__;}}while(0)
static long call(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
struct native_stat { uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,reserved;uint32_t links; };
_Static_assert(sizeof(struct native_stat)==24,"native subset layout");
_Static_assert(_Alignof(struct native_stat)==8,"native subset alignment");
_Static_assert(offsetof(struct native_stat,type)==18,"type offset");
_Static_assert(offsetof(struct native_stat,links)==20,"links offset");
int main(void){
 const char *a="/state/users/1000/DIR1", *b="/state/users/1000/DIR2";
 CHECK(!call(62,(long)a,0700,0));
 int fd=open(a,O_RDONLY);CHECK(fd>=0);int duplicate=(int)call(45,fd,0,0);CHECK(duplicate>=0);
 struct native_stat st;CHECK(!call(48,fd,(long)&st,0)&&st.type==2);
 CHECK(!call(66,(long)a,(long)b,0));
 CHECK(!call(48,fd,(long)&st,0)&&st.type==2);
 CHECK(!call(48,duplicate,(long)&st,0)&&st.type==2);
 CHECK(!close(fd));CHECK(!close(duplicate));CHECK(call(48,fd,(long)&st,0)==-5);
 puts("PY7 directory subset PASS: open/dup/rename/fstat/close; pinned lookup/enumeration/removal NOT_RUN");
 return 0;
}
