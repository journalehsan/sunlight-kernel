// Private adaptation of libc++'s atomic primitives to compiler builtins.
// The order names match libc++ so vendored libc++abi sources compile
// unchanged. A compare-exchange failure order cannot be release or acq_rel;
// those requests are strengthened to acquire, which is valid because the
// failing path is a load.
namespace std {
enum __libcpp_atomic_order {
    _AO_Relaxed = __ATOMIC_RELAXED,
    _AO_Consume = __ATOMIC_CONSUME,
    _AO_Acquire = __ATOMIC_ACQUIRE,
    _AO_Release = __ATOMIC_RELEASE,
    _AO_Acq_Rel = __ATOMIC_ACQ_REL,
    _AO_Seq = __ATOMIC_SEQ_CST
};

template<class T>
T __libcpp_atomic_load(const T* p, __libcpp_atomic_order order = _AO_Seq) {
    return __atomic_load_n(p, order);
}

template<class T>
void __libcpp_atomic_store(T* p, T value, __libcpp_atomic_order order = _AO_Seq) {
    __atomic_store_n(p, value, order);
}

template<class T>
T __libcpp_atomic_exchange(T* p, T value, __libcpp_atomic_order order = _AO_Seq) {
    return __atomic_exchange_n(p, value, order);
}

template<class T>
bool __libcpp_atomic_compare_exchange(T* p, T* expected, T desired,
                                      __libcpp_atomic_order success,
                                      __libcpp_atomic_order failure) {
    if (failure == _AO_Release || failure == _AO_Acq_Rel)
        failure = _AO_Acquire;
    return __atomic_compare_exchange_n(p, expected, desired, false, success, failure);
}

template<class T> T __libcpp_atomic_add(T* p, T value) {
    return __atomic_add_fetch(p, value, __ATOMIC_ACQ_REL);
}
}
