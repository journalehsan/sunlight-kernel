#define _GNU_SOURCE
#include <time.h>
int main(void){struct timespec a,b,d={0,1000000}; return clock_gettime(CLOCK_REALTIME,&a)||clock_gettime(CLOCK_MONOTONIC,&b)||nanosleep(&d,0);}
