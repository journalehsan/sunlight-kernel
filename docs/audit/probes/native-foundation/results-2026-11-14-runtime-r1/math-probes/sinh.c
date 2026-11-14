extern double sinh(double);
void * volatile requirement=(void *)&sinh;
int main(void){return requirement==0;}
