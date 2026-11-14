extern double lgamma(double);
void * volatile requirement=(void *)&lgamma;
int main(void){return requirement==0;}
