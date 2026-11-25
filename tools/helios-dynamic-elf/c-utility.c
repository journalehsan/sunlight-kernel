#include "c-utility.h"

enum { INVALID = -22, FAULT = -14, OVERFLOW = -75, UNTERMINATED = -61 };

static long range(struct utility_buffer buffer, unsigned long address, unsigned long length)
{
    if (!length)
        return 0;
    if (length > ~0UL - address || buffer.length > ~0UL - buffer.base)
        return OVERFLOW;
    if (!buffer.base || address < buffer.base || address + length > buffer.base + buffer.length)
        return FAULT;
    return 0;
}

long helios_memset(struct utility_buffer buffer, unsigned long dst, int byte, unsigned long length)
{
    long result = range(buffer, dst, length);
    if (result)
        return result;
    unsigned char *bytes = (unsigned char *)dst;
    for (unsigned long i = 0; i < length; ++i)
        bytes[i] = (unsigned char)byte;
    return 0;
}

long helios_memcpy(struct utility_buffer buffer, unsigned long dst, unsigned long src, unsigned long length)
{
    long result = range(buffer, dst, length);
    if (result)
        return result;
    result = range(buffer, src, length);
    if (result)
        return result;
    if (length && dst < src + length && src < dst + length)
        return INVALID;
    for (unsigned long i = 0; i < length; ++i)
        ((unsigned char *)dst)[i] = ((const unsigned char *)src)[i];
    return 0;
}

long helios_strlen(struct utility_buffer buffer, unsigned long src, unsigned long limit)
{
    long result = range(buffer, src, limit);
    if (result)
        return result;
    for (unsigned long i = 0; i < limit; ++i)
        if (!((const unsigned char *)src)[i])
            return (long)i;
    return UNTERMINATED;
}

static long call(long number, unsigned long a, unsigned long b, unsigned long c)
{
    __asm__ volatile("syscall" : "+a"(number) : "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return number;
}

long helios_write(struct utility_buffer buffer, unsigned long src, unsigned long length, unsigned long fd)
{
    long result = range(buffer, src, length);
    if (result || !length)
        return result;
    return call(1, fd, src, length);
}

void helios_exit(int status)
{
    call(60, (unsigned int)status, 0, 0);
    __builtin_trap();
}
