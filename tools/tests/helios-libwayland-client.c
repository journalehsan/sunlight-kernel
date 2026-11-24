#define _GNU_SOURCE
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client.h"

enum { WIDTH = 256, HEIGHT = 160, STRIDE = WIDTH * 4, OFFSET = 4096,
       FRAME_BYTES = STRIDE * HEIGHT };

#ifdef SUNLIGHT_CANVAS
extern bool sunlight_wayland_paint(uint32_t *, uint32_t, uint32_t, uint32_t,
                                  uint32_t, uint32_t *);
enum { POOL = OFFSET + 2 * FRAME_BYTES };
#else
enum { POOL = 170509 };
#endif

static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *shell;
static int synced, configured, done;

#ifdef SUNLIGHT_CANVAS_INPUT
extern bool sunlight_wayland_button(int32_t, int32_t, uint32_t, uint32_t, uint32_t *);
static struct wl_seat *seat;
static struct wl_pointer *pointer;
static struct wl_surface *pointer_target;
static uint32_t enter_serial, counter;
static wl_fixed_t pointer_x, pointer_y;
static int clicked;

static void pointer_enter(void *data, struct wl_pointer *object, uint32_t serial,
                          struct wl_surface *surface, wl_fixed_t x, wl_fixed_t y)
{
    (void)data;
    (void)object;
    enter_serial = surface == pointer_target ? serial : 0;
    pointer_x = x;
    pointer_y = y;
    if (enter_serial)
        fprintf(stderr, "[WL-INPUT] enter surface=%u x_fixed=%d y_fixed=%d serial=%u\n",
                wl_proxy_get_id((struct wl_proxy *)surface), x, y, serial);
}

static void pointer_leave(void *data, struct wl_pointer *object, uint32_t serial,
                          struct wl_surface *surface)
{
    (void)data;
    (void)object;
    (void)serial;
    (void)surface;
    enter_serial = 0;
}

static void pointer_motion(void *data, struct wl_pointer *object, uint32_t time,
                           wl_fixed_t x, wl_fixed_t y)
{
    (void)time;
    (void)x;
    (void)y;
    pointer_leave(data, object, 0, NULL);
}

static void pointer_button(void *data, struct wl_pointer *object, uint32_t serial,
                           uint32_t time, uint32_t button, uint32_t state)
{
    (void)data;
    (void)object;
    (void)time;
    if (enter_serial != 2 || serial != 3 || clicked)
        return;
    if (!sunlight_wayland_button(pointer_x, pointer_y, button, state, &counter))
        return;
    clicked = 1;
    char message[128];
    int bytes = snprintf(message, sizeof message,
                         "[WL-INPUT] hit surface=%u x_fixed=%d y_fixed=%d button=%u state=%u serial=%u counter=%u\n",
                         wl_proxy_get_id((struct wl_proxy *)pointer_target),
                         pointer_x, pointer_y, button, state, serial, counter);
    if (bytes > 0 && (size_t)bytes < sizeof message)
        (void)write(STDERR_FILENO, message, (size_t)bytes);
}

static void pointer_axis(void *data, struct wl_pointer *object, uint32_t time,
                         uint32_t axis, wl_fixed_t value)
{
    (void)data;
    (void)object;
    (void)time;
    (void)axis;
    (void)value;
}

static void seat_capabilities(void *data, struct wl_seat *object, uint32_t capabilities)
{
    (void)data;
    static const struct wl_pointer_listener listener = {
        .enter = pointer_enter, .leave = pointer_leave, .motion = pointer_motion,
        .button = pointer_button, .axis = pointer_axis,
    };
    if (!(capabilities & WL_SEAT_CAPABILITY_POINTER))
        enter_serial = 0;
    else if (!pointer) {
        pointer = wl_seat_get_pointer(object);
        if (pointer)
            wl_pointer_add_listener(pointer, &listener, NULL);
    }
}

static const struct wl_seat_listener seat_listener = {.capabilities = seat_capabilities};
#endif

static void require(int ok, const char *stage)
{
    if (!ok) {
        fprintf(stderr, "[WL-CLIENT] FAIL %s (errno=%d)\n", stage, errno);
        exit(1);
    }
}

static void hold_for_capture(void)
{
    struct timespec start, now;
    const struct timespec pause = {.tv_nsec = 100000000};
    require(clock_gettime(CLOCK_MONOTONIC, &start) == 0, "capture start");
    do {
        require(nanosleep(&pause, NULL) == 0, "capture hold");
        require(clock_gettime(CLOCK_MONOTONIC, &now) == 0, "capture clock");
    } while ((now.tv_sec - start.tv_sec) * INT64_C(1000000000)
             + now.tv_nsec - start.tv_nsec < INT64_C(5000000000));
}

static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version)
{
    (void)data;
    (void)version;
    if (!strcmp(interface, wl_compositor_interface.name))
        compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 1);
    else if (!strcmp(interface, wl_shm_interface.name))
        shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, xdg_wm_base_interface.name))
        shell = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
#ifdef SUNLIGHT_CANVAS_INPUT
    else if (!strcmp(interface, wl_seat_interface.name)) {
        seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
        if (seat)
            wl_seat_add_listener(seat, &seat_listener, NULL);
    }
#endif
}

static void global_remove(void *data, struct wl_registry *registry, uint32_t name)
{
    (void)data;
    (void)registry;
    (void)name;
}

static void configure(void *data, struct xdg_surface *surface, uint32_t serial)
{
    (void)data;
    xdg_surface_ack_configure(surface, serial);
    configured = 1;
}

static void toplevel_configure(void *data, struct xdg_toplevel *toplevel,
                               int32_t width, int32_t height, struct wl_array *states)
{
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
    (void)states;
}

static void toplevel_close(void *data, struct xdg_toplevel *toplevel)
{
    (void)data;
    (void)toplevel;
    require(0, "unexpected toplevel close");
}

static void release(void *data, struct wl_buffer *buffer)
{
    (void)buffer;
    *(int *)data = 1;
}

static void frame_done(void *data, struct wl_callback *callback, uint32_t timestamp)
{
    (void)data;
    (void)timestamp;
    wl_callback_destroy(callback);
    done = 1;
}

static void sync_done(void *data, struct wl_callback *callback, uint32_t serial)
{
    (void)data;
    (void)serial;
    wl_callback_destroy(callback);
    synced = 1;
}

static void dispatch(struct wl_display *display, const char *stage)
{
    const struct timespec timeout = {.tv_nsec = 100000000};
    require(wl_display_dispatch_timeout(display, &timeout) >= 0, stage);
}

static void wait_for(struct wl_display *display, const int *ready,
                     const int *also_ready, const char *stage)
{
    for (unsigned attempt = 0; attempt < 150; ++attempt) {
        if (*ready && (!also_ready || *also_ready))
            return;
        dispatch(display, stage);
    }
    require(*ready && (!also_ready || *also_ready), stage);
}

int main(void)
{
    static const struct sockaddr_un address = {
        .sun_family = AF_UNIX, .sun_path = "/sunlight/native-unix/wayland-fixture"
    };
    static const struct wl_registry_listener registry_listener = {global, global_remove};
    static const struct xdg_surface_listener surface_listener = {configure};
    static const struct xdg_toplevel_listener toplevel_listener = {
        .configure = toplevel_configure, .close = toplevel_close
    };
    static const struct wl_buffer_listener buffer_listener = {release};
    static const struct wl_callback_listener callback_listener = {frame_done};
    static const struct wl_callback_listener sync_listener = {sync_done};
#ifdef SUNLIGHT_CANVAS
    enum { BUFFER_COUNT = 2 };
#else
    enum { BUFFER_COUNT = 1 };
    static const uint32_t colors[][2] = {{0x28dcff, 0x125dd4}, {0xffcb32, 0xd42409}};
#endif
    static const struct timespec retry = {.tv_nsec = 100000000};

    fprintf(stderr, "[WL-LIB] entered main\n");
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    require(fd >= 0, "socket");
    int status = -1;
    for (unsigned attempt = 0; attempt < 150 && status != 0; ++attempt) {
        status = connect(fd, (const struct sockaddr *)&address, sizeof address);
        if (status != 0)
            nanosleep(&retry, NULL);
    }
    require(status == 0, "connect");
    struct wl_display *display = wl_display_connect_to_fd(fd);
    require(display != NULL, "wl_display_connect_to_fd");
    struct wl_registry *registry = wl_display_get_registry(display);
    require(registry && wl_registry_add_listener(registry, &registry_listener, NULL) == 0,
            "registry listener");
    struct wl_callback *sync = wl_display_sync(display);
    require(sync && wl_callback_add_listener(sync, &sync_listener, NULL) == 0, "registry sync");
    wait_for(display, &synced, NULL, "registry dispatch");
    require(compositor && shm && shell, "registry");
    fprintf(stderr, "[WL-LIB] discovered compositor, shm and xdg-shell\n");

    struct wl_surface *surface = wl_compositor_create_surface(compositor);
    require(surface != NULL, "create surface");
#ifdef SUNLIGHT_CANVAS_INPUT
    require(seat != NULL, "missing wl_seat global");
    pointer_target = surface;
#endif
    struct xdg_surface *xdg = xdg_wm_base_get_xdg_surface(shell, surface);
    require(xdg && xdg_surface_add_listener(xdg, &surface_listener, NULL) == 0,
            "xdg surface");
    struct xdg_toplevel *toplevel = xdg_surface_get_toplevel(xdg);
    require(toplevel && xdg_toplevel_add_listener(toplevel, &toplevel_listener, NULL) == 0,
            "xdg toplevel");
    wl_surface_commit(surface);
    wait_for(display, &configured, NULL, "initial configure");
#ifdef SUNLIGHT_CANVAS_INPUT
    require(pointer != NULL, "missing wl_pointer");
#endif
    require(wl_display_flush(display) >= 0, "flush ack");
    fprintf(stderr, "[WL-LIB] acknowledged configure\n");

    fd = memfd_create("wl-lib-probe", MFD_CLOEXEC);
    require(fd >= 0 && ftruncate(fd, POOL) == 0, "memfd size");
    void *map = mmap(NULL, POOL, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    require(map != MAP_FAILED, "mmap");
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, POOL);
    require(pool && wl_display_flush(display) >= 0, "create_pool flush");
    close(fd);
    struct wl_buffer *buffers[BUFFER_COUNT];
    int released[BUFFER_COUNT];
    for (unsigned slot = 0; slot < BUFFER_COUNT; ++slot) {
        released[slot] = 1;
        buffers[slot] = wl_shm_pool_create_buffer(
            pool, OFFSET + slot * FRAME_BYTES, WIDTH, HEIGHT, STRIDE, WL_SHM_FORMAT_XRGB8888);
        require(buffers[slot] && wl_buffer_add_listener(buffers[slot], &buffer_listener,
                                                       &released[slot]) == 0,
                "buffer listener");
    }
    for (unsigned frame = 0; frame < 2; ++frame) {
        unsigned slot = frame % BUFFER_COUNT;
        require(released[slot], "buffer available for paint");
        uint32_t *pixels = (uint32_t *)((char *)map + OFFSET + slot * FRAME_BYTES);
#ifdef SUNLIGHT_CANVAS
        uint32_t checksum;
#ifdef SUNLIGHT_CANVAS_INPUT
        unsigned paint = 2 + counter;
#else
        unsigned paint = frame;
#endif
        require(sunlight_wayland_paint(pixels, WIDTH, HEIGHT, STRIDE / 4, paint, &checksum),
                "Canvas XRGB8888 layout");
        fprintf(stderr, "[WL-CANVAS] frame=%u buffer=%u hash=%08x stride=%u XRGB8888\n",
                frame + 1, wl_proxy_get_id((struct wl_proxy *)buffers[slot]), checksum, STRIDE);
#else
        for (unsigned y = 0; y < HEIGHT; ++y)
            for (unsigned x = 0; x < WIDTH; ++x)
                pixels[y * WIDTH + x] = x >= 104 && x < 152 && y >= 60 && y < 100
                    ? 0xffffff : colors[frame][((frame ? y : x) / 16) % 2];
#endif
        released[slot] = done = 0;
        wl_surface_attach(surface, buffers[slot], 0, 0);
        wl_surface_damage(surface, 0, 0, WIDTH, HEIGHT);
        struct wl_callback *callback = wl_surface_frame(surface);
        require(callback && wl_callback_add_listener(callback, &callback_listener, NULL) == 0,
                "frame listener");
        wl_surface_commit(surface);
        wait_for(display, &released[slot], &done, "frame dispatch");
        fprintf(stderr, "[WL-LIB] frame %u released and callback completed\n", frame + 1);
        hold_for_capture();
#ifdef SUNLIGHT_CANVAS_INPUT
        if (frame == 0) {
            synced = 0;
            sync = wl_display_sync(display);
            require(sync && wl_callback_add_listener(sync, &sync_listener, NULL) == 0,
                    "pointer fixture sync listener");
            wait_for(display, &clicked, &synced, "pointer button dispatch / hit test");
        }
#endif
    }
#ifdef SUNLIGHT_CANVAS_INPUT
    wl_pointer_destroy(pointer);
    wl_seat_destroy(seat);
#endif
    wl_display_disconnect(display);
    munmap(map, POOL);
    return 0;
}
