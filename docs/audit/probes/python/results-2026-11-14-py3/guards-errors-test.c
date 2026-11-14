#include <sys/mman.h>
#include <string.h>
#include <stdlib.h>
#include <locale.h>
#include <wchar.h>
#include <errno.h>
#include <stdio.h>
#include <stdint.h>
#define CHECK(x) do{if(!(x)){printf("GUARD FAIL %d errno=%d\n",__LINE__,errno);return __LINE__;}}while(0)
struct native_stat {uint64_t size;uint32_t uid,gid;uint16_t mode;uint8_t type,pad;uint32_t nlinks;};
static long raw_stat(const char *path,struct native_stat *s){long r;__asm__ volatile("syscall":"=a"(r):"a"(61L),"D"(path),"S"(s):"rcx","r11","memory");return r;}
int main(void){
 char *s=mmap(0,8192,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);CHECK(s!=MAP_FAILED);CHECK(!munmap(s+4096,4096));
 wchar_t w[8];char b[16];CHECK(setlocale(LC_CTYPE,"C.UTF-8"));
 s[4095]=0;CHECK(mbstowcs(w,s+4095,8)==0&&w[0]==0);CHECK(mbstowcs(0,s+4095,0)==0);
 s[4094]=(char)0xc3;s[4095]=0;errno=0;CHECK(mbstowcs(w,s+4094,8)==(size_t)-1&&errno==EILSEQ);
 s[4095]='X';CHECK(mbstowcs(w,s+4095,1)==1&&w[0]=='X');CHECK(mbstowcs(w,(char*)1,0)==0);
 wchar_t *last=(wchar_t*)(s+4092);*last=0x1f600;memset(b,0x55,sizeof b);CHECK(wcstombs(b,last,4)==4&&(unsigned char)b[0]==0xf0&&b[4]==0x55);CHECK(wcstombs(b,last,3)==0&&b[4]==0x55);CHECK(wcstombs(b,(wchar_t*)1,0)==0);
 *last=0;CHECK(wcstombs(b,last,1)==0&&b[0]==0);CHECK(!munmap(s,4096));
 struct native_stat st;CHECK(raw_stat("/tmp/py3-no-such-parent/file",&st)==-3);CHECK(raw_stat("tmp",&st)==-6);CHECK(raw_stat("/tmp/../tmp",&st)==-6);CHECK(raw_stat("/tmp/.",&st)==-6);
 puts("PY3 GUARDS/RAW ERRORS PASS no overread; ENOENT=-3 EINVAL=-6 native wire");return 0;
}
