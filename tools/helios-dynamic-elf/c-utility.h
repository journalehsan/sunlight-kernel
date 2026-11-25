#ifndef HELIOS_C_UTILITY_H
#define HELIOS_C_UTILITY_H

#define HELIOS_C_DSO_PATH "/Applications/elf-c-dso.sunapp/Program/probe"

/* The trusted startup caller supplies a live, resolved allocation by value. */
struct utility_buffer {
    unsigned long base;
    unsigned long length;
};

long helios_memset(struct utility_buffer buffer, unsigned long dst, int byte, unsigned long length);
long helios_memcpy(struct utility_buffer buffer, unsigned long dst, unsigned long src, unsigned long length);
long helios_strlen(struct utility_buffer buffer, unsigned long src, unsigned long limit);
long helios_write(struct utility_buffer buffer, unsigned long src, unsigned long length, unsigned long fd);
void __attribute__((noreturn)) helios_exit(int status);

#endif
