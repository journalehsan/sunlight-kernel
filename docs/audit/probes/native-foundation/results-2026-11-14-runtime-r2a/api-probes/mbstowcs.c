#include <wchar.h>
void * volatile required_symbol = (void *)&mbstowcs;
int main(void) { return required_symbol == 0; }
