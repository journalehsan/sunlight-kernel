static __thread volatile unsigned initialized = 0x31;
static __thread volatile unsigned tail;

unsigned helios_tls_value(unsigned delta)
{
    unsigned before = initialized;
    unsigned zero = tail;
    initialized = before + delta;
    tail = zero + delta + 0x51;
    return (before << 24) | (zero << 16) | (initialized << 8) | tail;
}
