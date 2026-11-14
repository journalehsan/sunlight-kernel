#include <pthread.h>
void * volatile required_symbol = (void *)&pthread_attr_setstacksize;
int main(void) { return required_symbol == 0; }
