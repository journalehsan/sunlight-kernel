extern double log(double);
void * volatile requirement=(void *)&log;
int main(void){return requirement==0;}
