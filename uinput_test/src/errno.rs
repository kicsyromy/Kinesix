use std::os::raw::*;
use std::ffi::CStr;
use std::str::FromStr;

extern "C"
{
    #[no_mangle]
    fn __xpg_strerror_r(errnum: c_int, buf: *mut c_char, len: usize) -> i32;

    #[no_mangle]
    fn __errno_location() -> *mut i32;
}

pub fn errno() -> i32 {
    unsafe { *__errno_location() as i32 }
}

pub fn strerror(errnum: i32) -> String {
    let mut bufv: Vec<u8> = Vec::new();
    bufv.resize(256, 0);
    let len = bufv.len() - 1;

    let mut result: String = String::from("Unknown error");

    unsafe {
        let buf = bufv.as_mut_ptr() as *mut c_char;
        let r = __xpg_strerror_r(errnum, buf, len);

        if r == 0 {
            result = String::from_utf8_unchecked(bufv);
        }
    }

    result
}
