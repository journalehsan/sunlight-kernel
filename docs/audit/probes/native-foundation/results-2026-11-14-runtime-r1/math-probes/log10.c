extern double log10(double);
void * volatile requirement=(void *)&log10;
int main(void){return requirement==0;}
