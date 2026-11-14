/* Negative probe: the native C ABI must not link a dup2 call. */
int dup2(int old_fd, int new_fd);
int main(void) { return dup2(0, 1) == 1 ? 0 : 1; }
