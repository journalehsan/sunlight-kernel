#include <wchar.h>
void * volatile required_symbol = (void *)&mbrtowc;
int main(void) { return required_symbol == 0; }
