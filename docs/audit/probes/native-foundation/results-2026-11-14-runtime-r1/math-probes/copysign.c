extern double copysign(double);
void * volatile requirement=(void *)&copysign;
int main(void){return requirement==0;}
