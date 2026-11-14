#include <stdlib.h>
#include <stdint.h>
#include <stdio.h>
struct result { long a; double b; long c; };
__attribute__((noinline)) static struct result abi(long a,long b,long c,long d,long e,long f,long g,long h,double x,double y,double z,double u,double v,double w,double q,double r,double s) {
    struct result result={a+b+c+d+e+f+g+h,x+y+z+u+v+w+q+r+s,h};return result;
}
int main(void) {
    unsigned char *p=malloc(64); if (!p || (uintptr_t)p%16) return 1;
    for (int i=0;i<64;i++) p[i]=(unsigned char)i;
    unsigned char *q=realloc(p,1024); if (!q) {free(p);return 2;}
    for (int i=0;i<64;i++) if(q[i]!=(unsigned char)i) return 3;
    p=realloc(q,32);if(!p){free(q);return 4;}
    for(int i=0;i<32;i++) if(p[i]!=(unsigned char)i) return 5;
    free(p);free(0);
    p=calloc(32,8);if(!p) return 6;
    for(int i=0;i<256;i++) if(p[i]) return 7;
    free(p);
    struct result result=abi(1,2,3,4,5,6,7,8,1,2,3,4,5,6,7,8,9);
    if(result.a!=36 || result.b!=45 || result.c!=8) return 8;
    puts("R1 heap and SysV ABI PASS");return 0;
}
