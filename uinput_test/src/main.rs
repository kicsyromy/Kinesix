mod errno;
mod evdev_uinput;

use errno::*;
use ::std::os::raw::*;
use evdev_uinput::*;
use std::borrow::BorrowMut;
use std::process::exit;
use std::thread::sleep;
use std::time::Duration;

const LIBEVDEV_UINPUT_OPEN_MANAGED: i32 = -2;

#[link(name = "evdev")]
extern "C" {
    #[no_mangle]
    fn open(path: *const c_char, flags: i32, _: ...) -> i32;

    #[no_mangle]
    fn close(fd: i32) -> i32;

    #[no_mangle]
    fn write(fd: i32, buf: *const c_void, count: usize) -> __ssize_t;

    #[no_mangle]
    fn memset(object: *mut c_void, value: i32, count: usize) -> *mut c_void;

    #[no_mangle]
    fn strcpy(destination: *mut c_char, source: *const c_char) -> *mut c_char;

    #[no_mangle]
    fn strncpy(destination: *mut c_char, source: *const c_char, length: usize) -> *mut c_char;
}

fn main() {
    unsafe {
        let device = libevdev_new();
        libevdev_set_name(device, "virtkbd".as_ptr() as *const c_char);

        libevdev_enable_event_type(device, EV_KEY);
        libevdev_enable_event_code(device, EV_KEY, KEY_A, 0 as *const c_void);
        libevdev_enable_event_code(device, EV_KEY, KEY_LEFTSHIFT, 0 as *const c_void);

        let mut uidev= 0 as *mut libevdev_uinput;
        let err = libevdev_uinput_create_from_device(device, LIBEVDEV_UINPUT_OPEN_MANAGED, uidev.borrow_mut() as *mut *mut libevdev_uinput);
        if err != 0 {
            exit(err);
        }

        sleep(Duration::new(5, 0));

        let err = libevdev_uinput_write_event(uidev, EV_KEY, KEY_LEFTSHIFT, 1);
        let err = libevdev_uinput_write_event(uidev, EV_SYN, SYN_REPORT, 0);
        let err = libevdev_uinput_write_event(uidev, EV_KEY, KEY_A, 1);
        let err = libevdev_uinput_write_event(uidev, EV_SYN, SYN_REPORT, 0);
        let err = libevdev_uinput_write_event(uidev, EV_KEY, KEY_A, 0);
        let err = libevdev_uinput_write_event(uidev, EV_SYN, SYN_REPORT, 0);
        let err = libevdev_uinput_write_event(uidev, EV_KEY, KEY_LEFTSHIFT, 0);
        let err = libevdev_uinput_write_event(uidev, EV_SYN, SYN_REPORT, 0);

        sleep(Duration::new(1, 0));

        libevdev_uinput_destroy(uidev);
    }
}
