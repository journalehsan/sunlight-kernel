#include <sys/random.h>
void * volatile required_symbol = (void *)&getrandom;
int main(void) { return required_symbol == 0; }
