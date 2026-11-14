#include <stdio.h>
void * volatile required_symbol = (void *)&fseek;
int main(void) { return required_symbol == 0; }
