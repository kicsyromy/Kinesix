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
use std::borrow::{BorrowMut, Borrow};

const LIBEVDEV_UINPUT_OPEN_MANAGED: i32 = -2;

#[link(name = "evdev")]
extern "C" { }

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Key
{
    A = KEY_A as isize, B = KEY_B as isize, C = KEY_C as isize, D = KEY_D as isize,
    E = KEY_E as isize, F = KEY_F as isize, G = KEY_G as isize, H = KEY_H as isize,
    I = KEY_I as isize, J = KEY_J as isize, K = KEY_K as isize, L = KEY_L as isize,
    M = KEY_M as isize, N = KEY_N as isize, O = KEY_O as isize, P = KEY_P as isize,
    Q = KEY_Q as isize, R = KEY_R as isize, S = KEY_S as isize, T = KEY_T as isize,
    U = KEY_U as isize, V = KEY_V as isize, W = KEY_W as isize, X = KEY_X as isize,
    Y = KEY_Y as isize, Z = KEY_Z as isize,
    One = KEY_1 as isize,
    Two = KEY_2 as isize,
    Three = KEY_3 as isize,
    Four = KEY_4 as isize,
    Five = KEY_5 as isize,
    Six = KEY_6 as isize,
    Seven = KEY_7 as isize,
    Eight = KEY_8 as isize,
    Nine = KEY_9 as isize,
    Zero = KEY_0 as isize,
    F1 = KEY_F1 as isize, F2 = KEY_F2 as isize, F3 = KEY_F3 as isize, F4 = KEY_F4 as isize,
    F5 = KEY_F5 as isize, F6 = KEY_F6 as isize, F7 = KEY_F7 as isize, F8 = KEY_F8 as isize,
    F9 = KEY_F9 as isize, F10 = KEY_F10 as isize, F11 = KEY_F11 as isize, F12 = KEY_F12 as isize,
    LeftControl = KEY_LEFTCTRL as isize,
    LeftShift = KEY_LEFTSHIFT as isize,
    LeftAlt = KEY_LEFTALT as isize,
    LeftMeta = KEY_LEFTMETA as isize,
    RightControl = KEY_RIGHTCTRL as isize,
    RightShift = KEY_RIGHTSHIFT as isize,
    RightAlt = KEY_RIGHTALT as isize,
    RightMeta = KEY_RIGHTMETA as isize,
    Space = KEY_SPACE as isize,
    Tab = KEY_TAB as isize,
    Enter = KEY_ENTER as isize,
    CapsLock = KEY_CAPSLOCK as isize,
    PageUp = KEY_PAGEUP as isize,
    PageDown = KEY_PAGEDOWN as isize,
    LeftArrow = KEY_LEFT as isize,
    RightArrow = KEY_RIGHT as isize,
    UpArrow = KEY_UP as isize,
    DownArrow = KEY_DOWN as isize,
    Slash = KEY_SLASH as isize,
    Backslash = KEY_BACKSLASH as isize,
    Backspace = KEY_BACKSPACE as isize,
    Comma = KEY_COMMA as isize,
    Period = KEY_DOT as isize,
    Semicolon = KEY_SEMICOLON as isize,
    Apostrophe = KEY_APOSTROPHE as isize,
    Minus = KEY_MINUS as isize,
    Equals = KEY_EQUAL as isize,
    Backquote = KEY_GRAVE as isize,
    Escape = KEY_ESC as isize,
}

pub struct VirtualInput
{
    evdev_dev: *mut libevdev,
    virtual_device_name: String,
    uinput_dev: *mut libevdev_uinput
}

impl VirtualInput {
    pub fn new(device_name: &str) -> Result<VirtualInput, String> {
        let mut instance = VirtualInput {
            evdev_dev: 0 as *mut libevdev,
            virtual_device_name: device_name.to_string() + "\0",
            uinput_dev: 0 as *mut libevdev_uinput
        };

        unsafe {
            instance.evdev_dev = libevdev_new();
            libevdev_set_name(
                instance.evdev_dev,
                instance.virtual_device_name.as_ptr() as *const c_char
            );

            libevdev_enable_event_type(instance.evdev_dev, EV_KEY);

            /* Enable all keys for device */
            for i in 1..249 {
                libevdev_enable_event_code(instance.evdev_dev, EV_KEY, i as u32, 0 as *const c_void);
            }

            let err = libevdev_uinput_create_from_device(
                instance.evdev_dev,
                LIBEVDEV_UINPUT_OPEN_MANAGED,
                instance.uinput_dev.borrow_mut() as *mut *mut libevdev_uinput
            );
            if err != 0 {
                return Err(strerror(-err));
            }
        }

        Ok(instance)
    }

    fn press_release(&mut self, keys: &[Key], press: bool) -> Result<(), String> {
        unsafe {
            for key in keys {
                let err = libevdev_uinput_write_event(self.uinput_dev, EV_KEY, (*key) as u32, press as i32);
                if err != 0 {
                    return Err(strerror(-err));
                }
                let err = libevdev_uinput_write_event(self.uinput_dev, EV_SYN, SYN_REPORT, 0);
                if err != 0 {
                    return Err(strerror(-err));
                }
            }

            Ok(())
        }
    }

    pub fn press(&mut self, keys: &[Key], release: bool) -> Result<(), String> {
        let press_result = self.press_release(keys, true);
        if !press_result.is_ok() { return press_result; }
        let release_result = self.press_release(keys, false);
        if !release_result.is_ok() { return release_result; }

        Ok(())
    }

    pub fn release(&mut self, keys: &[Key]) -> Result<(), String> {
        self.press_release(keys, false)
    }
}

impl Drop for VirtualInput {
    fn drop(&mut self) {
        unsafe {
            libevdev_uinput_destroy(self.uinput_dev);
        }
    }
}
