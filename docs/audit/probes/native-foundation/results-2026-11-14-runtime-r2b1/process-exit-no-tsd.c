#include <pthread.h>
#include <stdlib.h>
#include <stdio.h>
static void destructor(void *p) { (void)p; puts("UNEXPECTED TSD DESTRUCTOR ON PROCESS EXIT"); }
int main(void) { pthread_key_t key; if(pthread_key_create(&key,destructor) || pthread_setspecific(key,(void *)1)) return 1; puts("PROCESS EXIT WITHOUT TSD CLEANUP"); exit(0); }
