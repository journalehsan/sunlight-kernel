#define _GNU_SOURCE
#include <signal.h>
static volatile sig_atomic_t seen; static void h(int s){seen=s;}
int main(void){struct sigaction a={0}; a.sa_handler=h; sigemptyset(&a.sa_mask); if(sigaction(SIGUSR1,&a,0)||raise(SIGUSR1))return 1; return seen!=SIGUSR1;}
