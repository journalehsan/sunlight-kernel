extern double tgamma(double);
void * volatile requirement=(void *)&tgamma;
int main(void){return requirement==0;}
