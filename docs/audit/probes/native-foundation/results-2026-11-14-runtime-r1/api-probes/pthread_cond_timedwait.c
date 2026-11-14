#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_cond_timedwait;
int main(void) { return required_symbol == 0; }
