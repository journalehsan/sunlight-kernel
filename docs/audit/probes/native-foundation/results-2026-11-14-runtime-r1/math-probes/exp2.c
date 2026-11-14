extern double exp2(double);
void * volatile requirement=(void *)&exp2;
int main(void){return requirement==0;}
