#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_mutex_init;
int main(void) { return required_symbol == 0; }
