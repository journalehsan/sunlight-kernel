#include <pthread.h>
#include <stdlib.h>
#include <stdio.h>
#include <time.h>
static void bad(void){puts("FAIL atexit ran");}
static void *worker(void *arg){(void)arg;puts("PY1 worker _Exit(37)");_Exit(37);}
int main(void){pthread_t t;if(atexit(bad)||pthread_create(&t,0,worker,0))return 91;
 struct timespec d={1,0};nanosleep(&d,0);puts("FAIL owner survived");return 92;}
