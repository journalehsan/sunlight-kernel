#define _GNU_SOURCE
#include <locale.h>
int main(void){return setlocale(LC_ALL,"C")==0;}
