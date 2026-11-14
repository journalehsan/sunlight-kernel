#include <wchar.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <locale.h>
#define CHECK(x) do {if(!(x)){printf("WIDE FAIL line=%d\n",__LINE__);return 1;}}while(0)
static int cmp(const void *a,const void *b) {int x=*(const int*)a,y=*(const int*)b;return (x>y)-(x<y);}
static int cmp3(const void *a,const void *b) {return memcmp(a,b,3);}
int main(void) {
 wchar_t b[32],*e,*save;const wchar_t *s=L"a\u03b1\U0001f642a";
 CHECK(sizeof(wchar_t)==4 && wcslen(s)==4);CHECK(wcscpy(b,s)==b && !wcscmp(b,s));
 CHECK(wcsncpy(b,L"x",5)==b && b[0]=='x' && b[1]==0 && b[4]==0);
 b[3]=123;CHECK(wcsncpy(b,L"abcdef",3)==b && b[0]=='a' && b[2]=='c' && b[3]==123);
 CHECK(wcsncmp(L"ab",L"ac",1)==0 && wcsncmp(L"ab",L"ac",2)<0 && wcscmp(L"z",L"a")>0);
 CHECK(wcschr(s,'a')==s && wcsrchr(s,'a')==s+3 && wcschr(s,0)==s+4 && !wcschr(s,'z'));
 CHECK(wmemchr(s,0,4)==0 && wmemchr(s,0,5)==s+4);
 wcscpy(b,L",a,,b;");CHECK(!wcscmp(wcstok(b,L",;",&save),L"a"));CHECK(!wcscmp(wcstok(0,L",;",&save),L"b"));CHECK(!wcstok(0,L",;",&save));
 CHECK(wcstol(L" -0x7f!",&e,0)==-127 && *e=='!');errno=0;CHECK(wcstol(L"9999999999999999999999",&e,10)==LONG_MAX && errno==ERANGE);
 char *end;CHECK(strtoul("-1!",&end,10)==ULONG_MAX && *end=='!');errno=0;CHECK(strtoul("18446744073709551616",&end,10)==ULONG_MAX && errno==ERANGE);
 CHECK(getenv("TMPDIR") && !strcmp(getenv("TMPDIR"),"/tmp"));CHECK(!getenv("PY2_MISSING") && !getenv("") && !getenv("x=y"));
 char out[64];
 CHECK(setlocale(LC_CTYPE,"C"));CHECK(snprintf(out,sizeof out,"%ls",L"plain")==5 && !strcmp(out,"plain"));
 errno=0;CHECK(snprintf(out,sizeof out,"%ls",L"\u03b1")==-1 && errno==EILSEQ);
 CHECK(setlocale(LC_CTYPE,"C.UTF-8"));
 CHECK(snprintf(out,sizeof out,"%8.3ls",L"a\u03b1\U0001f642")==8 && !strcmp(out,"     a\xce\xb1"));
 CHECK(snprintf(out,sizeof out,"%.2ls",L"a\u03b1")==1 && !strcmp(out,"a"));
 CHECK(snprintf(out,sizeof out,"%lc",(wint_t)0x1f642)==4 && !memcmp(out,"\xf0\x9f\x99\x82",5));
 CHECK(snprintf(out,sizeof out,"%.0ls",L"\u03b1")==0 && !out[0]);
 int values[257];
 for(int size=0;size<=257;size++) {
  for(int i=0;i<size;i++)values[i]=((i*137+size*19)%31)-15;
  qsort(values,(size_t)size,sizeof values[0],cmp);
  for(int i=1;i<size;i++)CHECK(values[i-1]<=values[i]);
  int sum=0,expected=0;for(int i=0;i<size;i++){sum+=values[i];expected+=((i*137+size*19)%31)-15;}CHECK(sum==expected);
 }
 unsigned char triples[][3]={{3,2,1},{0,2,9},{3,2,0},{1,0,0}};qsort(triples,4,3,cmp3);CHECK(triples[0][0]==0 && triples[1][0]==1 && triples[2][2]==0 && triples[3][2]==1);
 puts("PY2 WIDE/ENV/SORT PASS");return 0;
}
