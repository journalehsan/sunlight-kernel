#include <stdlib.h>
void * volatile required_symbol = (void *)&setenv;
int main(void) { return required_symbol == 0; }
