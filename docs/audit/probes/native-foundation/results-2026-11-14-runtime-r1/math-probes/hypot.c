extern double hypot(double);
void * volatile requirement=(void *)&hypot;
int main(void){return requirement==0;}
