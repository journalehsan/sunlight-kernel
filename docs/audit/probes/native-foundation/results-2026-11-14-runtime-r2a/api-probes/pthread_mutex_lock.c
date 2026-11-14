#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_mutex_lock;
int main(void) { return required_symbol == 0; }
