/* Diagnostic synthetic declaration ONLY, not a supported public API. */
long double frexpl(long double,int *);
int main(void){ int ex=0; long double ip=0; (void)ex;(void)ip; return frexpl(0.25L,&ex) != 0;}
