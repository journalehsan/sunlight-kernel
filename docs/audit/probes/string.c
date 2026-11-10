#define _GNU_SOURCE
#include <string.h>
int main(void){char b[8]; memcpy(b,"abc",4); return strcmp(b,"abc");}
