extern double fmod(double);
void * volatile requirement=(void *)&fmod;
int main(void){return requirement==0;}
