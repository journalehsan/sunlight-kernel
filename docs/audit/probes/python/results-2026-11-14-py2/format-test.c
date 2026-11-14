#include <stdio.h>
#include <string.h>
#include <stdarg.h>
#include <stdint.h>
#include <limits.h>
#include <errno.h>
#include "format-oracle.h"
#define CHECK(x) do { if (!(x)) { printf("FAIL line=%d case=%lu\n",__LINE__,(unsigned long)index);return 1;} } while(0)
static int via_vsn(char *b,size_t n,const char *f,...) {va_list a;va_start(a,f);int r=vsnprintf(b,n,f,a);va_end(a);return r;}
static int via_vs(char *b,const char *f,...) {va_list a;va_start(a,f);int r=vsprintf(b,f,a);va_end(a);return r;}
int main(void) {
 size_t index=0;char b[4096];
 CHECK(snprintf(b,sizeof b,"")==0 && b[0]==0);
 CHECK(snprintf(0,0,"%020d",3)==20);
 CHECK(snprintf(b,1,"abc")==3 && b[0]==0);
 CHECK(snprintf(b,3,"abcd")==4 && !memcmp(b,"ab\0",3));
 CHECK(sprintf(b,"%hhd %hu %ld %llu %jd %zu %td",-128,65535,LONG_MIN,ULLONG_MAX,(intmax_t)-9,(size_t)17,(ptrdiff_t)-11)>0);
 CHECK(!strcmp(b,"-128 65535 -9223372036854775808 18446744073709551615 -9 17 -11"));
 CHECK(via_vsn(b,sizeof b,"%#08x|%*.*s|%#.0o|%+.0d",42,-5,2,"abcd",0,0)==18);
 CHECK(!strcmp(b,"0x00002a|ab   |0|+"));
 CHECK(via_vs(b,"%s:%d","ok",7)==4 && !strcmp(b,"ok:7"));
 CHECK(snprintf(b,sizeof b,"%100000d",7)==100000 && b[4095]==0);
 CHECK(sprintf(b,"%s","caller capacity")==15);
 for(index=0;index<sizeof floats/sizeof *floats;index++) {
  union { unsigned long long u;double d; } v={.u=floats[index].bits};
  int n=snprintf(b,sizeof b,floats[index].fmt,v.d);
  if(n!=floats[index].length || strcmp(b,floats[index].expected)) {printf("FLOAT FAIL case=%lu format=%s n=%d expected=%d\n",(unsigned long)index,floats[index].fmt,n,floats[index].length);return 2;}
  char small[9];memset(small,0x5a,sizeof small);
  CHECK(snprintf(small,8,floats[index].fmt,v.d)==n);
  CHECK(small[8]==0x5a && small[n<7?n:7]==0 && !memcmp(small,b,n<7?(size_t)n:7));
 }
 errno=0;CHECK(snprintf(b,sizeof b,"%a",1.0)==-1 && errno==ENOSYS);
 errno=0;CHECK(snprintf(b,sizeof b,"%2147483648d",1)==-1 && errno==EOVERFLOW);
 printf("PY2 FORMAT PASS oracle=%lu\n",(unsigned long)index);return 0;
}
