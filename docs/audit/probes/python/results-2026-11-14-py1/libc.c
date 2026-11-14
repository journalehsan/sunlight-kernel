#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#define CHECK(x) do { if (!(x)) return __LINE__; } while(0)
int main(void) {
 for(size_t n=0;n<9;n++)for(size_t len=0;len<9;len++) {
  char src[10]; char dst[11];memset(src,'a',sizeof src);src[len]=0;memset(dst,0x5a,sizeof dst);
  CHECK(strncpy(dst+1,src,n)==dst+1);CHECK(dst[0]==0x5a);CHECK(dst[n+1]==0x5a);
  for(size_t i=0;i<n;i++)CHECK(dst[i+1]==(i<len?'a':0));
 }
 char b[16];CHECK(strcpy(b,"hello")==b);CHECK(!strcmp(b,"hello"));
 CHECK(strcspn("abcxyz","xy")==3);CHECK(strcspn("abc","")==3);CHECK(strcspn("","a")==0);
 const char *s="abcxyz";CHECK(strpbrk(s,"yx")==s+3);CHECK(!strpbrk(s,""));
 CHECK(strstr(s,"cxy")==s+2);CHECK(strstr(s,"")==s);CHECK(!strstr(s,"xyzq"));CHECK(!strstr("","q"));
 char *end;errno=0;CHECK(strtol("-0x8000000000000000!",&end,0)==(-9223372036854775807L-1));CHECK(*end=='!'&&errno==0);
 errno=0;CHECK(strtol("9223372036854775808",&end,10)==9223372036854775807L&&errno==ERANGE);
 CHECK(strtol(" +077z",&end,0)==63&&*end=='z');
 puts("PY1 strings and strtol PASS");
 puts("PY1 strncpy PASS: 81 boundary cases and canaries");return 0;
}
