#ifndef HELIOS_PAGE_ALLOCATOR_H
#define HELIOS_PAGE_ALLOCATOR_H

/* x86-64 Size4KiB anonymous mmap ABI. */
enum { PAGE_BYTES = 4096, PAGE_SLOTS = 8, PAGE_SINGLE_MAX = 8, PAGE_TOTAL_MAX = 16 };
enum { PAGE_INVALID = -22, PAGE_NO_MEMORY = -12, PAGE_PUBLICATION = -5 };

struct page_handle {
    unsigned long owner, generation, base, pages, slot;
};

struct page_allocator {
    unsigned long owner, generation, used, pages;
    struct page_handle slots[PAGE_SLOTS];
};

/* Map returns a positive guest base or negative errno. Unmap returns 0 or
 * negative errno. The optional gate runs just before metadata publication.
 * fatal must terminate the process if an unpublished mapping cannot be undone.
 */
struct page_backend {
    long (*map)(unsigned long bytes);
    long (*unmap)(unsigned long base, unsigned long bytes);
    int (*publish)(unsigned long base, unsigned long bytes);
    __attribute__((noreturn)) void (*fatal)(void);
};

static inline unsigned long page_resolve(const struct page_allocator *allocator,
                                         struct page_handle handle)
{
    if (!allocator->owner || handle.owner != allocator->owner ||
        handle.slot >= PAGE_SLOTS || !handle.generation)
        return 0;
    const struct page_handle *live = &allocator->slots[handle.slot];
    if (live->generation != handle.generation || live->base != handle.base ||
        live->pages != handle.pages)
        return 0;
    return live->base;
}

static inline long page_alloc(struct page_allocator *allocator,
                              const struct page_backend *backend,
                              unsigned long pages, struct page_handle *out)
{
    unsigned long bytes;
    if (!allocator->owner || !pages ||
        __builtin_mul_overflow(pages, (unsigned long)PAGE_BYTES, &bytes) ||
        pages > PAGE_SINGLE_MAX)
        return PAGE_INVALID;
    if (pages > PAGE_TOTAL_MAX - allocator->pages || allocator->used == PAGE_SLOTS ||
        allocator->generation == ~0UL)
        return PAGE_NO_MEMORY;

    unsigned long slot = 0;
    while (allocator->slots[slot].generation)
        ++slot;
    long base = backend->map(bytes);
    if (base < 0)
        return base;
    if (!base || ((unsigned long)base & (PAGE_BYTES - 1)))
        backend->fatal();
    if (backend->publish && !backend->publish((unsigned long)base, bytes)) {
        if (backend->unmap((unsigned long)base, bytes))
            backend->fatal();
        return PAGE_PUBLICATION;
    }

    struct page_handle handle = {
        allocator->owner, allocator->generation + 1, (unsigned long)base, pages, slot
    };
    allocator->slots[slot] = handle;
    allocator->generation = handle.generation;
    allocator->pages += pages;
    ++allocator->used;
    *out = handle;
    return 0;
}

static inline long page_free(struct page_allocator *allocator,
                             const struct page_backend *backend, struct page_handle handle)
{
    if (!page_resolve(allocator, handle))
        return PAGE_INVALID;
    long result = backend->unmap(handle.base, handle.pages * PAGE_BYTES);
    if (result)
        return result;

    allocator->slots[handle.slot] = (struct page_handle){0};
    allocator->pages -= handle.pages;
    --allocator->used;
    return 0;
}

#endif
