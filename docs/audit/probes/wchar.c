#define _GNU_SOURCE
#include <wchar.h>
#include <locale.h>
int main(void){setlocale(LC_ALL,"C"); wchar_t w=0; return mbrtowc(&w,"A",1,0)!=1||w!=L'A';}
