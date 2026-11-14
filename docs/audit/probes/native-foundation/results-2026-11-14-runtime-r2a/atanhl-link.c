/* Diagnostic synthetic declaration ONLY, not a supported public API. */
long double atanhl(long double);
int main(void){ int ex=0; long double ip=0; (void)ex;(void)ip; return atanhl(0.25L) != 0;}
