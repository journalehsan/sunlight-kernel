#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <time.h>
static void bad(void){puts("FAIL callback");}
static void *worker(void *p){(void)p;puts("PY3 worker _exit(38)");_exit(38);}
int main(void){pthread_t t;if(atexit(bad)||pthread_create(&t,0,worker,0))return 91;struct timespec d={1,0};nanosleep(&d,0);puts("FAIL owner survived");return 92;}
