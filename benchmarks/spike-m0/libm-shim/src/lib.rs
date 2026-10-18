//! The C maths functions that `rnx2rtkp` imports, implemented by the `libm`
//! crate (a Rust port of musl's maths library). Loading this into a native
//! glibc build with `LD_PRELOAD` replaces glibc's maths library and nothing
//! else, which shows whether differences between builds come from it.

macro_rules! unary {
    ($($name:ident),*) => {
        $(#[unsafe(no_mangle)] pub extern "C" fn $name(x: f64) -> f64 { libm::$name(x) })*
    };
}

macro_rules! binary {
    ($($name:ident),*) => {
        $(#[unsafe(no_mangle)] pub extern "C" fn $name(x: f64, y: f64) -> f64 { libm::$name(x, y) })*
    };
}

unary!(acos, asin, atan, cos, exp, log10, log2, round, sin, sqrt, tan);
binary!(atan2, fmod, pow);

/// # Safety
/// `s` and `c` must be valid for writes, as C's `sincos` requires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sincos(x: f64, s: *mut f64, c: *mut f64) {
    let (sin, cos) = libm::sincos(x);
    // SAFETY: the caller passes valid pointers, as for C's sincos.
    unsafe {
        *s = sin;
        *c = cos;
    }
}
