#define _GNU_SOURCE
#include <sys/random.h>
int main(void){char b[16]; return getrandom(b,sizeof b,0)!=sizeof b;}
