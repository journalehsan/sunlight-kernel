extern double sqrt(double);
void * volatile requirement=(void *)&sqrt;
int main(void){return requirement==0;}
