extern double atan2(double);
void * volatile requirement=(void *)&atan2;
int main(void){return requirement==0;}
