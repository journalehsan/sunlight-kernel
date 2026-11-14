#include <time.h>
void * volatile required_symbol = (void *)&nanosleep;
int main(void) { return required_symbol == 0; }
