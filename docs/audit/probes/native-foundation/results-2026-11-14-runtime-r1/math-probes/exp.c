extern double exp(double);
void * volatile requirement=(void *)&exp;
int main(void){return requirement==0;}
