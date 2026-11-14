#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_cond_wait;
int main(void) { return required_symbol == 0; }
