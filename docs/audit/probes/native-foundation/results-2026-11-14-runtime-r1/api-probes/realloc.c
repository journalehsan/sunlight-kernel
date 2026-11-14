#include <stdlib.h>
void * volatile required_symbol = (void *)&realloc;
int main(void) { return required_symbol == 0; }
