#include <unistd.h>
void * volatile required_symbol = (void *)&fork;
int main(void) { return required_symbol == 0; }
