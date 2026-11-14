extern double cosh(double);
void * volatile requirement=(void *)&cosh;
int main(void){return requirement==0;}
