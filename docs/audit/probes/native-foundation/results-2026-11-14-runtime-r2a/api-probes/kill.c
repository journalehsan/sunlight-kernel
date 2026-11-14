#include <signal.h>
void * volatile required_symbol = (void *)&kill;
int main(void) { return required_symbol == 0; }
