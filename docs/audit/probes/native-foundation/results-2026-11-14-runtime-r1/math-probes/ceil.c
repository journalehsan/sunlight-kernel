extern double ceil(double);
void * volatile requirement=(void *)&ceil;
int main(void){return requirement==0;}
