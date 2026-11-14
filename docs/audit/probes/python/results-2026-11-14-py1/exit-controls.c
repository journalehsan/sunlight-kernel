#include <pthread.h>
#include <stdlib.h>
#include <stdio.h>
#include <time.h>
static void bad(void){puts("FAIL callback ran");}
static void *peer(void *arg){(void)arg;struct timespec t={1,0};nanosleep(&t,0);puts("FAIL peer survived");return 0;}
int main(int argc,char **argv){if(argc!=2)return 91;pthread_t t;if(atexit(bad)||pthread_create(&t,0,peer,0))return 92;puts("PY1 initial owner immediate group exit");_Exit((int)strtol(argv[1],0,10));}
