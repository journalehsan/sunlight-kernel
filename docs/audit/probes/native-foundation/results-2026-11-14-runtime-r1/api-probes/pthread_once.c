#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_once;
int main(void) { return required_symbol == 0; }
