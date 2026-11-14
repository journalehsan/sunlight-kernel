#include <stdio.h>
void * volatile required_symbol = (void *)&fwrite;
int main(void) { return required_symbol == 0; }
