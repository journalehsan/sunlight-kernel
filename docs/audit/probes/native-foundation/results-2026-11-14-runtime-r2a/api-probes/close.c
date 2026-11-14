#include <unistd.h>
void * volatile required_symbol = (void *)&close;
int main(void) { return required_symbol == 0; }
