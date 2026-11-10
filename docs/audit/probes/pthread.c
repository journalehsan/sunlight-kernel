#define _GNU_SOURCE
#include <pthread.h>
static _Thread_local int tls=7; static void *worker(void *p){(void)p; return tls==7 ? 0 : (void*)1;}
int main(void){pthread_t t; void *v; if(pthread_create(&t,0,worker,0))return 1; return pthread_join(t,&v)||v!=0;}
