#include <stdio.h>
void * volatile required_symbol = (void *)&snprintf;
int main(void) { return required_symbol == 0; }
