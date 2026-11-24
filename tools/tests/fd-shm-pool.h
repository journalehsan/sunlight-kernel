enum { SMALL_POOL = 4096, LARGE_POOL = 8192 };

static void put_pattern(volatile unsigned char *view, unsigned length, unsigned seed)
{
    for (unsigned index = 0; index < length; ++index)
        view[index] = (unsigned char)(seed + index * 19 + (index >> 12));
}

static void check_pattern(volatile unsigned char *view, unsigned length, unsigned seed)
{
    for (unsigned index = 0; index < length; ++index)
        require(view[index] == (unsigned char)(seed + index * 19 + (index >> 12)),
                "[FD-SHARED] FAIL pool pattern");
}

static volatile unsigned char *map_pool(long fd, unsigned length, unsigned offset)
{
    long result = syscall6(MMAP, 0, length, 3, 1, fd, offset);
    require(result > 0, "[FD-SHARED] FAIL mmap pool");
    return (volatile unsigned char *)result;
}

static void close_fd(long fd)
{
    require(call(CLOSE, fd, 0, 0) == 0, "[FD-SHARED] FAIL close pool fixture FD");
}

static void unmap_pool(volatile unsigned char *view, unsigned length)
{
#ifdef NATIVE
    const int number = 51;
#else
    const int number = 11;
#endif
    require(call(number, (long)view, length, 0) == 0, "[FD-SHARED] FAIL unmap pool");
}

#ifdef NATIVE
static long accept_stream(long listener)
{
    long stream;
    do {
        stream = call(184, listener, 0, 0);
        if (stream == -11) call(YIELD, 0, 0, 0);
    } while (stream == -11);
    require(stream >= 0, "[FD-SHARED] FAIL accept pool stream");
    return stream;
}

static long receive_pool(long stream, unsigned id, unsigned length)
{
    unsigned char request[16] = {0};
    long received = call(185, stream, 0, 0);
    require(received >= 0 && received >> 32 == 7, "[FD-SHARED] FAIL receive pool FD");
    request[0] = 7;
    for (unsigned used = 1; used < sizeof request;) {
        long count = call(READ, stream, (long)(request + used), sizeof request - used);
        require(count > 0, "[FD-SHARED] FAIL read pool request");
        used += (unsigned)count;
    }
    const unsigned expected[] = {7, 16U << 16, id, length};
    for (unsigned index = 0; index < sizeof request; ++index)
        require(request[index] == ((const unsigned char *)expected)[index],
                "[FD-SHARED] FAIL pool request header");
    return (int)received;
}

static void run(void)
{
    long listener = call(183, (long)name, sizeof name - 1, 0);
    require(listener >= 0, "[FD-SHARED] FAIL listen for pools");
    long coordination = accept_stream(listener);
    expect_stage(coordination, 'A');
    long stream = accept_stream(listener);
    long large_fd = receive_pool(stream, 8, LARGE_POOL);
    require(syscall6(MMAP, 0, LARGE_POOL + 1, 3, 1, large_fd, 0) == INVALID &&
            syscall6(MMAP, 0, SMALL_POOL, 3, 1, large_fd, 1) == INVALID,
            "[FD-SHARED] FAIL memfd bounds");
    volatile unsigned char *large = map_pool(large_fd, LARGE_POOL, 0);
    volatile unsigned char *tail = map_pool(large_fd, SMALL_POOL, SMALL_POOL);
    close_fd(large_fd);
    check_pattern(large, LARGE_POOL, 31);
    check_pattern(tail, SMALL_POOL, 32);
    unmap_pool(tail, SMALL_POOL);
    send_stage(coordination, 'B');
    expect_stage(coordination, 'C');
    long small_fd = receive_pool(stream, 9, SMALL_POOL);
    volatile unsigned char *small = map_pool(small_fd, SMALL_POOL, 0);
    close_fd(small_fd);
    check_pattern(small, SMALL_POOL, 173);
    check_pattern(large, LARGE_POOL, 31);
    put_pattern(large, LARGE_POOL, 97);
    check_pattern(small, SMALL_POOL, 173);
    send_stage(coordination, 'D');
    expect_stage(coordination, 'E');
    check_pattern(large, LARGE_POOL, 221);
    check_pattern(small, SMALL_POOL, 173);
    unmap_pool(large, LARGE_POOL);
    unmap_pool(small, SMALL_POOL);
    log_line("[MEMFD-TEST] independent mappings released after FD close");
    send_stage(coordination, 'G');
    expect_stage(coordination, 'F');
    close_fd(stream);
    log_line("[MEMFD-TEST] PASS");
}
#else
static long connect_stream(void)
{
    struct { unsigned short family; char path[108]; } address = {1, {0}};
    for (unsigned index = 0; index < sizeof name; ++index) address.path[index] = name[index];
    long stream = call(41, 1, 1, 0);
    require(stream >= 0, "[FD-SHARED] FAIL pool socket");
    long result;
    do {
        result = call(42, stream, (long)&address, sizeof name + 2);
        if (result == -111) call(YIELD, 0, 0, 0);
    } while (result == -111);
    require(result == 0, "[FD-SHARED] FAIL connect pool stream");
    return stream;
}

static long create_pool(unsigned size)
{
#ifdef HOST_TEMP_POOL
    static const char temp[] = "wl-shm-measurement.tmp";
    long fd = syscall6(257, -100, (long)temp, 0x800c2, 0600, 0, 0);
    require(fd >= 0 && call(263, -100, (long)temp, 0) == 0,
            "[FD-SHARED] FAIL temporary pool");
#else
    long fd = call(319, (long)"wl-shm-pool", 1, 0);
#endif
    require(fd >= 0 && call(77, fd, size, 0) == 0, "[FD-SHARED] FAIL size pool");
    return fd;
}

static void send_pool(long stream, int fd, unsigned id, unsigned size)
{
    const unsigned request[] = {7, 16U << 16, id, size};
    struct { const void *base; unsigned long len; } iov = {request, sizeof request};
    struct { unsigned long len; int level, type, fd, padding; } control = {20, 1, 1, fd, 0};
    struct {
        void *name;
        unsigned name_len;
        const void *iov;
        unsigned long iov_len;
        const void *control;
        unsigned long control_len;
        int flags;
    } msg = {0, 0, &iov, 1, &control, sizeof control, 0};
    require(call(46, stream, (long)&msg, 0x4000) == sizeof request,
            "[FD-SHARED] FAIL send pool request");
}

#if defined(HOST_POOL_PROBE) || defined(HOST_TEMP_POOL)
static void run(void)
{
    long stream = connect_stream();
    long fd = create_pool(LARGE_POOL);
    volatile unsigned char *view = map_pool(fd, LARGE_POOL, 0);
    put_pattern(view, LARGE_POOL, 31);
    send_pool(stream, fd, 8, LARGE_POOL);
    close_fd(fd);
    send_stage(stream, 'A');
    expect_stage(stream, 'B');
    check_pattern(view, LARGE_POOL, 97);
    unmap_pool(view, LARGE_POOL);
    close_fd(stream);
}
#else
static void run(void)
{
    long coordination = connect_stream();
    long stream = connect_stream();
    long large_fd = create_pool(LARGE_POOL);
    long small_fd = create_pool(SMALL_POOL);
    volatile unsigned char *large = map_pool(large_fd, LARGE_POOL, 0);
    volatile unsigned char *small = map_pool(small_fd, SMALL_POOL, 0);
    require(large[4095] == 0 && large[4096] == 0 && small[0] == 0,
            "[FD-SHARED] FAIL pool initialization");
    require(call(77, large_fd, LARGE_POOL * 2, 0) == -1 &&
            call(77, large_fd, SMALL_POOL, 0) == -1,
            "[FD-SHARED] FAIL pool resize");
    put_pattern(large, LARGE_POOL, 31);
    put_pattern(small, SMALL_POOL, 173);
    send_pool(stream, large_fd, 8, LARGE_POOL);
    close_fd(large_fd);
    send_stage(coordination, 'A');
    expect_stage(coordination, 'B');
    send_pool(stream, small_fd, 9, SMALL_POOL);
    close_fd(small_fd);
    send_stage(coordination, 'C');
    expect_stage(coordination, 'D');
    check_pattern(large, LARGE_POOL, 97);
    check_pattern(small, SMALL_POOL, 173);
    put_pattern(large, LARGE_POOL, 221);
    unmap_pool(large, LARGE_POOL);
    unmap_pool(small, SMALL_POOL);
    send_stage(coordination, 'E');
    expect_stage(coordination, 'G');
    long discarded_fd = create_pool(LARGE_POOL);
    send_pool(stream, discarded_fd, 10, LARGE_POOL);
    close_fd(discarded_fd);
    close_fd(stream);
    send_stage(coordination, 'F');
}
#endif
#endif
