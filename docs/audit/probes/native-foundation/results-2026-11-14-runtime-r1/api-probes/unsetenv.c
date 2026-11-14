#include <stdlib.h>
void * volatile required_symbol = (void *)&unsetenv;
int main(void) { return required_symbol == 0; }
