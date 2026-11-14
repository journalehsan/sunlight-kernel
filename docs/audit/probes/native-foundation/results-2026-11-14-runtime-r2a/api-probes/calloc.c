#include <stdlib.h>
void * volatile required_symbol = (void *)&calloc;
int main(void) { return required_symbol == 0; }
