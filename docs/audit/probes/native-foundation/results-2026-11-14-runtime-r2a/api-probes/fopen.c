#include <stdio.h>
void * volatile required_symbol = (void *)&fopen;
int main(void) { return required_symbol == 0; }
