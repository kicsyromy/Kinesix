
/*
 * Copyright © 2019 Romeo Calota
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the licence, or (at your option) any later version.
 *
 * This software is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this program; if not, see <http://www.gnu.org/licenses/>.
 *
 * Author: Romeo Calota
 */

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

fn test() {
    unsafe {
        let device = libevdev_new();
        libevdev_set_name(device, "virtkbd\0".as_ptr() as *const c_char);

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
