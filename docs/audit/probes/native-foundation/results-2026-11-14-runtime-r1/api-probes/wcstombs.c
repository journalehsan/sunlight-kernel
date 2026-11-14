#include <wchar.h>
void * volatile required_symbol = (void *)&wcstombs;
int main(void) { return required_symbol == 0; }
