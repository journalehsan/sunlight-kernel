#define _GNU_SOURCE
#include <dlfcn.h>
int main(void){void *h=dlopen(0,RTLD_NOW); if(!h)return 1; int bad=dlsym(h,"malloc")==0; dlclose(h); return bad;}
