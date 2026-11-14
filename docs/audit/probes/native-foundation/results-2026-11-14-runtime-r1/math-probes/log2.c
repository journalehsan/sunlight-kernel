extern double log2(double);
void * volatile requirement=(void *)&log2;
int main(void){return requirement==0;}
