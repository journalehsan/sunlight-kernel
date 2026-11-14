#define _GNU_SOURCE
#include <math.h>
#include <errno.h>
#include <stdio.h>
#include <stdint.h>
#include "math-vectors.h"
static double value(unsigned long u) { union { unsigned long u; double d; } v={u};return v.d; }
static unsigned long bits(double d) { union { double d; unsigned long u; } v={d};return v.u; }
static unsigned int bitsf(float d) { union { float d; unsigned int u; } v={d};return v.u; }
static double absolute(double x) {return x<0?-x:x;}
static int exact(double x,double y) {return isnan(y)?isnan(x):bits(x)==bits(y);}
static int near(double x,double y,int kind) {
 if (isnan(y)) return isnan(x);
 if (isinf(y)||y==0 || absolute(y)<0x1p-1022) return exact(x,y);
 if (!isfinite(x)) return 0;
 if(kind==0) { unsigned long a=bits(x),b=bits(y);return signbit(x)==signbit(y) && (a>b?a-b:b-a)<=4; }
 if(kind==4) {unsigned int a=bitsf((float)x),b=bitsf((float)y);return signbit(x)==signbit(y) && (a>b?a-b:b-a)<=4;}
 return absolute(x-y) <= (kind==1?8e-14:4e-6)*(absolute(y)>1?absolute(y):1);
}
int main(void) {
 unsigned long n=sizeof vectors/sizeof *vectors;
 for(unsigned long j=0;j<n;j++) {
  struct vector v=vectors[j];double x=value(v.x),y=value(v.y),z=value(v.z),r=0,i=0;int ex=999,ok=0;float fi;
  switch(v.kind) {
   case 0:r=atanh(x);ok=near(r,y,0);break;
   case 1:r=lgamma_r(x,&ex);ok=near(r,y,1) && (!isfinite(y)||ex==v.exponent);break;
   case 2:r=frexp(x,&ex);ok=exact(r,y)&&(!isfinite(x)||ex==v.exponent);break;
   case 3:r=modf(x,&i);ok=exact(r,y)&&exact(i,z);break;
   case 4:r=atanhf((float)x);ok=near(r,y,4);break;
   case 5:r=lgammaf((float)x);ok=near(r,y,5);break;
   case 6:r=frexpf((float)x,&ex);ok=exact(r,y)&&(!isfinite(x)||ex==v.exponent);break;
   case 7:r=modff((float)x,&fi);i=fi;ok=exact(r,y)&&exact(i,z);break;
  }
  if(!ok) {printf("FAIL vector=%lu kind=%d input=%lx actual=%lx expected=%lx exponent=%d expected_exp=%d\n",j,v.kind,v.x,bits(r),v.y,ex,v.exponent);return 1;}
 }
 errno=0;volatile double r=atanh(1);if(!isinf(r)||errno!=ERANGE)return 2;
 errno=0;r=atanh(-1);if(!isinf(r)||!signbit(r)||errno!=ERANGE)return 3;
 errno=0;r=atanh(2);if(!isnan(r)||errno!=EDOM)return 4;
 errno=0;r=lgamma(-2);if(!isinf(r)||errno!=ERANGE)return 5;
 errno=0;r=lgamma(1e308);if(!isinf(r)||errno!=ERANGE)return 6;
 errno=77;r=atanh(.25);if(errno!=77)return 7;
 errno=77;r=lgamma(3.5);if(errno!=77)return 8;
 r=lgamma(-.5);if(signgam!=-1)return 9;
 r=lgamma(-1.5);if(signgam!=1)return 10;
 /* Observe actual SSE flags without advertising an unsupported fenv API. */
 unsigned old,clear,flags;__asm__ volatile("stmxcsr %0":"=m"(old));clear=old&~63U;
 __asm__ volatile("ldmxcsr %0"::"m"(clear));r=atanh(2);__asm__ volatile("stmxcsr %0":"=m"(flags));if(!(flags&1))return 11;
 __asm__ volatile("ldmxcsr %0"::"m"(clear));r=atanh(1);__asm__ volatile("stmxcsr %0":"=m"(flags));if(!(flags&4))return 12;
 __asm__ volatile("ldmxcsr %0"::"m"(old));
 printf("R2A math PASS vectors=%lu errno/signgam/SSE-domain-pole PASS\n",n);return 0;
}
