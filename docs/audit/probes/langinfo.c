#define _GNU_SOURCE
#include <langinfo.h>
int main(void){return nl_langinfo(CODESET)==0;}
