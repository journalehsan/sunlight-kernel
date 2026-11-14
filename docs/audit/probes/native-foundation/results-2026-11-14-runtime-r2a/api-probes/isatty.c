#include <unistd.h>
void * volatile required_symbol = (void *)&isatty;
int main(void) { return required_symbol == 0; }
