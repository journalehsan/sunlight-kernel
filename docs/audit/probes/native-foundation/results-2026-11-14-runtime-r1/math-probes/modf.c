extern double modf(double);
void * volatile requirement=(void *)&modf;
int main(void){return requirement==0;}
