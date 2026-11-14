#include <stdio.h>
void * volatile required_symbol = (void *)&fclose;
int main(void) { return required_symbol == 0; }
