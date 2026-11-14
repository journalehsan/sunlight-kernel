extern double pow(double);
void * volatile requirement=(void *)&pow;
int main(void){return requirement==0;}
