#include <stdio.h>
void * volatile required_symbol = (void *)&fread;
int main(void) { return required_symbol == 0; }
