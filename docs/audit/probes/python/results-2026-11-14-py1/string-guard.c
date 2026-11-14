#include <sys/mman.h>
#include <string.h>
#include <stdio.h>
#define CHECK(x) do{if(!(x))return __LINE__;}while(0)
int main(void){char *s=mmap(0,8192,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);CHECK(s!=MAP_FAILED);CHECK(!munmap(s+4096,4096));char d[16];memset(d,0x5a,sizeof d);
 s[4095]=0;CHECK(strncpy(d,s+4095,16)==d);for(int i=0;i<16;i++)CHECK(d[i]==0);
 s[4095]='X';CHECK(strncpy(d,s+4095,1)==d);CHECK(d[0]=='X');CHECK(strncpy(s+4095,"Y",1)==s+4095);CHECK(s[4095]=='Y');CHECK(!munmap(s,4096));puts("PY1 strncpy guard-page PASS");return 0;}
