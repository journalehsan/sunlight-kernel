#include <stdlib.h>
void * volatile required_symbol = (void *)&free;
int main(void) { return required_symbol == 0; }
