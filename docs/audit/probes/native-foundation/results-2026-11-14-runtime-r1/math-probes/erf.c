extern double erf(double);
void * volatile requirement=(void *)&erf;
int main(void){return requirement==0;}
