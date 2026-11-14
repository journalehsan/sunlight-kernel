#include <pthread.h>
pthread_cond_t c;
int main(void){return pthread_cond_init(&c,0);}
