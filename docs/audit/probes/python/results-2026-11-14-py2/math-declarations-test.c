#include <math.h>
#include <stdio.h>
#define CHECK(x) do {if(!(x)){printf("MATH DECL FAIL %d\n",__LINE__);return 1;}}while(0)
int main(void) {
 double (*volatile unary[])(double)={ceil,cos,exp,floor,round,sin};
 double (*volatile binary[])(double,double)={atan2,copysign,fmod,hypot,pow};
 CHECK(unary[0](1.25)==2 && unary[0](-1.25)==-1);
 CHECK(unary[1](0)==1 && unary[1](1)>0.540302305868139 && unary[1](1)<0.540302305868141);
 CHECK(unary[2](0)==1 && unary[2](1)>2.71828182845904 && unary[2](1)<2.71828182845905);
 CHECK(unary[3](1.25)==1 && unary[3](-1.25)==-2);
 CHECK(unary[4](2.5)==3 && unary[4](-2.5)==-3);
 CHECK(unary[5](0)==0 && unary[5](1)>0.841470984807895 && unary[5](1)<0.841470984807898);
 CHECK(binary[0](0,1)==0 && binary[0](1,1)>0.785398163397447 && binary[0](1,1)<0.785398163397450);
 CHECK(binary[1](3,-1)==-3 && signbit(binary[1](0,-1)));
 CHECK(binary[2](5.5,2)==1.5 && binary[2](-5.5,2)==-1.5);
 CHECK(binary[3](3,4)==5 && binary[4](2,10)==1024 && binary[4](-2,3)==-8);
 puts("PY2 MATH DECL PASS native archive ABI/value checks");return 0;
}
