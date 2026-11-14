//! C ABI for pinned, vendored Rust libm algorithms. No long-double narrowing.
//! Domain/pole/overflow errno is supplied here; no public fenv API is claimed.
use crate::errno::set_errno;

#[no_mangle]
pub static mut signgam: i32 = 0;

fn atanh_error(x: f64) {
    if x.abs() > 1.0 {
        set_errno(33); // EDOM
    } else if x.abs() == 1.0 {
        set_errno(34); // ERANGE: pole
    }
}
#[no_mangle]
pub extern "C" fn atanh(x: f64) -> f64 {
    atanh_error(x);
    libm::atanh(x)
}
#[no_mangle]
pub extern "C" fn atanhf(x: f32) -> f32 {
    atanh_error(x as f64);
    libm::atanhf(x)
}
#[no_mangle]
pub unsafe extern "C" fn frexp(x: f64, exponent: *mut i32) -> f64 {
    let (fraction, exp) = libm::frexp(x);
    exponent.write(exp);
    fraction
}
#[no_mangle]
pub unsafe extern "C" fn frexpf(x: f32, exponent: *mut i32) -> f32 {
    let (fraction, exp) = libm::frexpf(x);
    exponent.write(exp);
    fraction
}
#[no_mangle]
pub unsafe extern "C" fn modf(x: f64, integral: *mut f64) -> f64 {
    let (fraction, integer) = libm::modf(x);
    integral.write(integer);
    fraction
}
#[no_mangle]
pub unsafe extern "C" fn modff(x: f32, integral: *mut f32) -> f32 {
    let (fraction, integer) = libm::modff(x);
    integral.write(integer);
    fraction
}
fn gamma_error(x: f64, y: f64) {
    if x.is_finite() && y.is_infinite() {
        set_errno(34); // pole or overflow
    }
}
#[no_mangle]
pub unsafe extern "C" fn lgamma_r(x: f64, sign: *mut i32) -> f64 {
    let (y, s) = libm::lgamma_r(x);
    sign.write(s);
    gamma_error(x, y);
    y
}
#[no_mangle]
pub unsafe extern "C" fn lgammaf_r(x: f32, sign: *mut i32) -> f32 {
    let (y, s) = libm::lgammaf_r(x);
    sign.write(s);
    gamma_error(x as f64, y as f64);
    y
}
#[no_mangle]
pub unsafe extern "C" fn lgamma(x: f64) -> f64 {
    lgamma_r(x, &raw mut signgam)
}
#[no_mangle]
pub unsafe extern "C" fn lgammaf(x: f32) -> f32 {
    lgammaf_r(x, &raw mut signgam)
}
