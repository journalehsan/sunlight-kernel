extern double ldexp(double);
void * volatile requirement=(void *)&ldexp;
int main(void){return requirement==0;}
