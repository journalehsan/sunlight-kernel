#include <stdio.h>
#include <wchar.h>
#include <locale.h>
#include <errno.h>
int main(void) {char b[64]={0};printf("locale=%s mb=%lu\n",setlocale(LC_CTYPE,"C.UTF-8"),(unsigned long)sizeof(wchar_t));int n=snprintf(b,sizeof b,"%8.3ls",L"a\u03b1\U0001f642");printf("n=%d errno=%d out=",n,errno);for(int i=0;i<12;i++)printf("%02x",(unsigned)(unsigned char)b[i]);printf("\n");return 0;}
