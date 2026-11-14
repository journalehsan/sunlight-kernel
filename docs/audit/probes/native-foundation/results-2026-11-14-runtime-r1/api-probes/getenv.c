#include <stdlib.h>
void * volatile required_symbol = (void *)&getenv;
int main(void) { return required_symbol == 0; }
