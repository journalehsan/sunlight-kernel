/* Diagnostic synthetic declaration ONLY, not a supported public API. */
char *strerror(int);
int main(void){ int ex=0; long double ip=0; (void)ex;(void)ip; return (strerror(1)!=0) != 0;}
