#include <stdlib.h>
void * volatile required_symbol = (void *)&malloc;
int main(void) { return required_symbol == 0; }
