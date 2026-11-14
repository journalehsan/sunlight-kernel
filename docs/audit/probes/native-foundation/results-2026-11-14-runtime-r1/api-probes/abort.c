#include <stdlib.h>
void * volatile required_symbol = (void *)&abort;
int main(void) { return required_symbol == 0; }
