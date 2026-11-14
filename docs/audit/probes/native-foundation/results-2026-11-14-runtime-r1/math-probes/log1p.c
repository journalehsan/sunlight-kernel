extern double log1p(double);
void * volatile requirement=(void *)&log1p;
int main(void){return requirement==0;}
