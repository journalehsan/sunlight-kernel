#include <signal.h>
void * volatile required_symbol = (void *)&sigaction;
int main(void) { return required_symbol == 0; }
