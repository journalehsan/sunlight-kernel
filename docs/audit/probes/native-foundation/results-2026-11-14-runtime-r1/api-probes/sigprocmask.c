#include <signal.h>
void * volatile required_symbol = (void *)&sigprocmask;
int main(void) { return required_symbol == 0; }
