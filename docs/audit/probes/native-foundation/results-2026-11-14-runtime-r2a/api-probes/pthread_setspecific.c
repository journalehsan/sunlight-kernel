#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_setspecific;
int main(void) { return required_symbol == 0; }
