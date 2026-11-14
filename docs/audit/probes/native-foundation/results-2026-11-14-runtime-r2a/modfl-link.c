/* Diagnostic synthetic declaration ONLY, not a supported public API. */
long double modfl(long double,long double *);
int main(void){ int ex=0; long double ip=0; (void)ex;(void)ip; return modfl(0.25L,&ip) != 0;}
