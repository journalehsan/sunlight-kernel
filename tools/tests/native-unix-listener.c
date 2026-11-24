static const char name[] = "/sunlight/native-unix/fixture";
static const char control[] = "/sunlight/native-unix/control";

static long call(long number, long first, long second, long third)
{
    __asm__ volatile("syscall" : "+a"(number)
                     : "D"(first), "S"(second), "d"(third)
                     : "rcx", "r11", "memory");
    return number;
}

#ifdef NATIVE
enum { READ = 42, WRITE = 43, CLOSE = 41, YIELD = 21, EXIT = 20, LOG = 99 };
#else
enum { READ = 0, WRITE = 1, CLOSE = 3, YIELD = 24, EXIT = 60,
       LOG = (1UL << 63) | 99 };
#endif

static void log_line(const char *message)
{
    long length = 0;
    while (message[length]) ++length;
    call(LOG, (long)message, length, 0);
}

static void require(int condition, const char *message)
{
    if (condition) return;
    log_line(message);
    call(EXIT, 1, 0, 0);
    __builtin_trap();
}

static void send_byte(long fd, char byte)
{
    require(call(WRITE, fd, (long)&byte, 1) == 1, "[UNIX-FIX] FAIL write");
}

static void expect_byte(long fd, char expected)
{
    char byte = 0;
    require(call(READ, fd, (long)&byte, 1) == 1 && byte == expected,
            "[UNIX-FIX] FAIL read or byte mismatch");
}

#ifdef NATIVE
static long listen_name(const char *path, long length, long uid)
{
    return call(183, (long)path, length, uid);
}

static long accept_stream(long listener)
{
    long fd;
    do {
        fd = call(184, listener, 0, 0);
        if (fd == -11) call(YIELD, 0, 0, 0);
    } while (fd == -11);
    require(fd >= 0, "[UNIX-FIX] FAIL accept");
    return fd;
}

static void run(void)
{
    if (call(35, 0, 0, 0) != 0) {
        require(listen_name(name, sizeof name - 1, 1000) == -13,
                "[UNIX-FIX] FAIL unauthorized registration");
        log_line("[UNIX-FIX] nonroot native registration denied");
        return;
    }
    long listener = listen_name(name, sizeof name - 1, 1000);
    require(listener >= 0, "[UNIX-FIX] FAIL register listener");
    require(listen_name(name, sizeof name - 1, 1000) == -98,
            "[UNIX-FIX] FAIL duplicate registration");
    long gate = listen_name(control, sizeof control - 1, 1001);
    require(gate >= 0, "[UNIX-FIX] FAIL register control");
    long denied = accept_stream(gate);
    expect_byte(denied, 'D');
    require(call(184, listener, 0, 0) == -11, "[UNIX-FIX] FAIL denied client queued");
    log_line("[UNIX-FIX] denied client left queue empty");
    send_byte(denied, 'A');
    static const char ready[] = "/sunlight/native-unix/ready";
    long ready_listener = listen_name(ready, sizeof ready - 1, 1000);
    require(ready_listener >= 0, "[UNIX-FIX] FAIL register ready");
    send_byte(accept_stream(ready_listener), 'G');
    long stream = accept_stream(listener);
    expect_byte(stream, 'Q');
    send_byte(stream, 'R');
    require(call(CLOSE, listener, 0, 0) == 0, "[UNIX-FIX] FAIL close");
    send_byte(stream, 'C');
    expect_byte(stream, 'F');
    long reopened = listen_name(name, sizeof name - 1, 1000);
    require(reopened >= 0, "[UNIX-FIX] FAIL name not released");
    require(call(CLOSE, reopened, 0, 0) == 0, "[UNIX-FIX] FAIL close rebound");
    log_line("[UNIX-FIX] PASS exchange, close, refusal and rebind");
}
#else
static long attempt(long fd, const char *path, long syscall)
{
    struct { unsigned short family; char path[108]; } address = {1, {0}};
    unsigned length = 0;
    while (path[length]) {
        address.path[length] = path[length];
        ++length;
    }
    return call(syscall, fd, (long)&address, length + 3);
}

static long socket(void)
{
    long fd = call(41, 1, 1, 0);
    require(fd >= 0, "[UNIX-FIX] FAIL Linux socket");
    return fd;
}

static long connect_wait(const char *path)
{
    long fd = socket();
    long result;
    do {
        result = attempt(fd, path, 42);
        if (result == -111) call(YIELD, 0, 0, 0);
    } while (result == -111);
    require(result == 0, "[UNIX-FIX] FAIL Linux connect");
    return fd;
}

static void run(void)
{
    long uid = call(102, 0, 0, 0);
    if (uid == 1001) {
        long fd = socket();
        require(attempt(fd, name, 49) == -13, "[UNIX-FIX] FAIL Linux bind reserved name");
        long gate = connect_wait(control);
        require(attempt(fd, name, 42) == -111, "[UNIX-FIX] FAIL wrong UID connect");
        log_line("[UNIX-FIX] wrong UID Linux connect returned ECONNREFUSED");
        send_byte(gate, 'D');
        expect_byte(gate, 'A');
        return;
    }
    require(uid == 1000, "[UNIX-FIX] FAIL allowed Linux UID");
    long gate = connect_wait("/sunlight/native-unix/ready");
    expect_byte(gate, 'G');
    long fd = connect_wait(name);
    send_byte(fd, 'Q');
    expect_byte(fd, 'R');
    expect_byte(fd, 'C');
    long next = socket();
    require(attempt(next, name, 42) == -111, "[UNIX-FIX] FAIL connect after close");
    log_line("[UNIX-FIX] post-close Linux connect returned ECONNREFUSED");
    send_byte(fd, 'F');
}
#endif

__attribute__((force_align_arg_pointer, noreturn))
void _start(void)
{
    run();
    call(EXIT, 0, 0, 0);
    __builtin_trap();
}
