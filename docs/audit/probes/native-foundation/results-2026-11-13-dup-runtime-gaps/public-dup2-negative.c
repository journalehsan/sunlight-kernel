extern int dup2(int, int);
int main(void) { return dup2(0, 1); }
