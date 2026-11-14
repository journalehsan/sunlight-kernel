//! Allocation-free C qsort. Heapsort bounds stack use and comparisons even for
//! sorted or duplicate-heavy input. Stability is not part of the C contract.
use core::ffi::{c_int, c_void};
type Compare = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;
unsafe fn swap(base: *mut u8, size: usize, a: usize, b: usize) {
    if a == b {
        return;
    }
    let a = base.add(a * size);
    let b = base.add(b * size);
    for i in 0..size {
        let value = a.add(i).read();
        a.add(i).write(b.add(i).read());
        b.add(i).write(value);
    }
}
unsafe fn sift(base: *mut u8, size: usize, mut root: usize, count: usize, compare: Compare) {
    while root < count / 2 {
        let mut child = root * 2 + 1;
        if child + 1 < count
            && compare(
                base.add(child * size).cast(),
                base.add((child + 1) * size).cast(),
            ) < 0
        {
            child += 1;
        }
        if compare(base.add(root * size).cast(), base.add(child * size).cast()) >= 0 {
            return;
        }
        swap(base, size, root, child);
        root = child;
    }
}
#[no_mangle]
pub unsafe extern "C" fn qsort(
    base: *mut c_void,
    count: usize,
    size: usize,
    compare: Option<Compare>,
) {
    if count < 2 || size == 0 {
        return;
    }
    if base.is_null()
        || compare.is_none()
        || count
            .checked_mul(size)
            .filter(|&n| n <= isize::MAX as usize)
            .is_none()
    {
        crate::errno::set_errno(crate::errno::EINVAL);
        return;
    }
    let compare = compare.unwrap();
    let base = base.cast::<u8>();
    for root in (0..count / 2).rev() {
        sift(base, size, root, count, compare);
    }
    for end in (2..=count).rev() {
        swap(base, size, 0, end - 1);
        sift(base, size, 0, end - 1, compare);
    }
}
