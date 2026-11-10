#define _GNU_SOURCE
#include <math.h>
int main(void){volatile double x=2.0; return fabs(sqrt(x)*sqrt(x)-x)>1e-12;}
