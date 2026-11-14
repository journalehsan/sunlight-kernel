#include <wchar.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#define CHECK(x) do {if(!(x)){printf("WIDE FAIL line=%d\n",__LINE__);return 1;}}while(0)
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
 CHECK(getenv("PY2_TEST") && !strcmp(getenv("PY2_TEST"),"startup-value"));CHECK(!getenv("PY2_MISSING") && !getenv("") && !getenv("x=y"));
 puts("PY2 WIDE/ENV PASS");return 0;
}
