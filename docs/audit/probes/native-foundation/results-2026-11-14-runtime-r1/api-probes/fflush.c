#include <stdio.h>
void * volatile required_symbol = (void *)&fflush;
int main(void) { return required_symbol == 0; }
