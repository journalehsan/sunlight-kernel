#include <time.h>
void * volatile required_symbol = (void *)&clock_gettime;
int main(void) { return required_symbol == 0; }
