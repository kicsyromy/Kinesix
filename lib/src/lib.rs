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

pub mod device;

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::str;
use std::sync::mpsc;
use std::time::Duration;

use libc;

use crate::device::Device;
use std::borrow::Borrow;
use std::ffi::{CStr, CString};

const POLLIN: libc::c_short = 0x1;

#[link(name = "mtdev")]
#[link(name = "evdev")]
#[link(name = "wacom")]
#[link(name = "input")]
extern "C" {
    #[no_mangle]
    fn exit(exit_code: libc::c_int) -> !;

    #[no_mangle]
    fn open(path: *const libc::c_char, flags: libc::c_int, _: ...) -> libc::c_int;

    #[no_mangle]
    fn close(fd: libc::c_int) -> libc::c_int;

    #[no_mangle]
    fn poll(path: *mut libc::pollfd, nfds: libc::nfds_t, timeout: libc::c_int) -> libc::c_int;

    #[no_mangle]
    fn strncpy(destination: *mut libc::c_char, source: *const libc::c_char, length: libc::size_t) -> *mut libc::c_char;

    #[no_mangle]
    fn g_timeout_add_full(priority: i32, interval: u32, fun: unsafe extern "C" fn(*mut libc::c_void) -> i32, data: *mut libc::c_void, notify: *mut libc::c_void) -> u32;

    #[no_mangle]
    fn libinput_get_fd(libinput: *mut libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_dispatch(handle: *const libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_unref(libinput: *mut libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_get_event(libinput: *mut libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_event_destroy(event: *mut libc::c_void);

    #[no_mangle]
    fn libinput_event_get_type(event: *mut libc::c_void) -> libinput::EventType;

    #[no_mangle]
    fn libinput_event_gesture_get_finger_count(gesture_event: *mut libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_event_gesture_get_cancelled(gesture_event: *mut libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_event_gesture_get_scale(gesture_event: *mut libc::c_void) -> f64;

    #[no_mangle]
    fn libinput_event_get_gesture_event(event: *mut libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_event_gesture_get_dx_unaccelerated(gesture_event: *mut libc::c_void) -> f64;

    #[no_mangle]
    fn libinput_event_gesture_get_dy_unaccelerated(gesture_event: *mut libc::c_void) -> f64;

    #[no_mangle]
    fn libinput_path_create_context(interface: *const libinput::Interface, user_data: *const libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_device_ref(device: *mut libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_device_unref(device: *mut libc::c_void) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_path_add_device(libinput: *mut libc::c_void, path: *const libc::c_char) -> *mut libc::c_void;

    #[no_mangle]
    fn libinput_path_remove_device(device: *mut libc::c_void);

    #[no_mangle]
    fn libinput_device_has_capability(device: *mut libc::c_void, capability: libinput::DeviceCapability) -> i32;

    #[no_mangle]
    fn libinput_device_get_name(device: *mut libc::c_void) -> *const libc::c_char;

    #[no_mangle]
    fn libinput_device_get_id_product(device: *mut libc::c_void) -> u32;

    #[no_mangle]
    fn libinput_device_get_id_vendor(device: *mut libc::c_void) -> u32;
}

mod libinput {
    #[allow(dead_code)]
    #[derive(Copy, Clone)]
    #[repr(C)]
    pub enum DeviceCapability {
        Keyboard = 0,
        Pointer = 1,
        Touch = 2,
        TabletTool = 3,
        TabletPad = 4,
        Gesture = 5,
        Switch = 6,
    }

    #[allow(dead_code)]
    #[derive(Copy, Clone)]
    #[repr(C)]
    pub enum EventType {
        None = 0,

        DeviceAdded,
        DeviceRemoved,

        KeyboardKey = 300,

        PointerMotion = 400,
        PointerMotionAbsolute,
        PointerButton,
        PointerAxis,

        TouchDown = 500,
        TouchUp,
        TouchMotion,
        TouchCancel,
        TouchFrame,

        TabletToolAxis = 600,
        TabletToolProximity,
        TabletToolTip,
        TabletToolButton,

        TabletPadButton = 700,
        TabletPadRing,

        TabletPadStrip,

        GestureSwipeBegin = 800,
        GestureSwipeUpdate,
        GestureSwipeEnd,
        GesturePinchBegin,
        GesturePinchUpdate,
        GesturePinchEnd,

        SwitchToggle = 900,
    }

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct Interface {
        pub open_restricted: Option<
            unsafe extern "C" fn(
                path: *const libc::c_char,
                flags: i32,
                user_data: *mut libc::c_void,
            ) -> i32,
        >,
        pub close_restricted: Option<
            unsafe extern "C" fn(fd: i32, user_data: *mut libc::c_void),
        >
    }
}

unsafe extern "C" fn open_restricted(path: *const libc::c_char, flags: i32, _: *mut libc::c_void) -> i32 {
    let fd;
    fd = open(path, flags);
    if fd == -1 {
        println!("Failed to open file descriptor.");
        exit(-1);
    }
    fd
}

unsafe extern "C" fn close_restricted(fd: i32, _: *mut libc::c_void) {
    close(fd);
}

#[derive(Debug)]
pub struct Input {
    pub interface: Box<libinput::Interface>,
    pub instance: *mut libc::c_void,
    pub active_device: *mut libc::c_void,

    /* The absolute maximum value for swipe velocity */
    /* These help determine swipe direction */
    pub swipe_x_max: f64,
    pub swipe_y_max: f64,
}

impl Input {
    pub fn new() -> Input {
        unsafe {
            let interface = Box::into_raw(Box::new(libinput::Interface {
                open_restricted: Some(open_restricted),
                close_restricted: Some(close_restricted)
            }));

            let mut self_ = Input {
                interface: Box::from_raw(0 as *mut libinput::Interface),
                instance: libinput_path_create_context(interface as *const libinput::Interface, 0 as *const libc::c_void),
                active_device: 0 as *mut libc::c_void,
                swipe_x_max: 0.0,
                swipe_y_max: 0.0,
            };
            self_.interface = Box::from_raw(interface);

            self_
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe {
            libinput_unref(self.instance);
        }
    }
}

pub struct EventPollerThread
{
    handle: Option<std::thread::JoinHandle<()>>,
    cancelation_token: mpsc::Sender<bool>,
    cancelation_requested: bool,
    libinput_event_listener: mpsc::Receiver<()>,
}

impl EventPollerThread {
    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("Failed to join event polling thread");
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SwipeDirection
{
    SwipeUp,
    SwipeDown,
    SwipeLeft,
    SwipeRight,
    None
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum PinchType
{
    PinchIn,
    PinchOut,
    None
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum GestureEventState
{
    Started,
    Ongoing,
    Finished,
    Unknown,
}


#[derive(Debug)]
pub enum GestureType
{
    Swipe(SwipeDirection),
    Pinch(PinchType),
    Unknown,
}

const DEVICES_PATH: &str = "/dev/input/";
const GESTURE_DELTA: f64 = 10.0;

pub struct KinesixBackend
{
    active_device: *const Device,
    valid_device_list: Vec<Device>,

    swipe_delegate: Box<dyn FnMut(SwipeDirection, i32)>,
    pinch_delegate: Box<dyn FnMut(PinchType, i32)>,

    ongoing_gesture_type: GestureType,
    input: Input,

    event_poller_thread: Option<EventPollerThread>,
}

impl KinesixBackend
{
    pub fn new<SwipeDelegate: 'static + FnMut(SwipeDirection, i32), PinchDelegate: 'static + FnMut(PinchType, i32)>(swipe_delegate: SwipeDelegate, pinch_delegate: PinchDelegate) -> KinesixBackend {
        KinesixBackend {
            active_device: std::ptr::null(),
            valid_device_list: Vec::new(),
            swipe_delegate: Box::new(swipe_delegate),
            pinch_delegate: Box::new(pinch_delegate),
            ongoing_gesture_type: GestureType::Unknown,
            input: Input::new(),
            event_poller_thread: None,
        }
    }

    fn create_device(&mut self, device_path: &str) -> Option<Device> {
        unsafe {
            // HACK: For some reason when passing a string into C land it has some junk on the end
            //       so we allocate a new buffer where we copy .len() bytes into and pass it along
            //       to the function that takes a char *
            let mut device_path_vec: Vec<libc::c_char> = Vec::new();
            device_path_vec.resize(device_path.len() + 1, 0);
            let device_path_cstr = strncpy(device_path_vec.as_mut_ptr(), device_path.as_ptr() as *const libc::c_char, device_path.len());

            let libinput_dev = libinput_path_add_device(self.input.instance, device_path_cstr as *const libc::c_char);
            if libinput_dev as usize != 0 {
                if libinput_device_has_capability(libinput_dev, libinput::DeviceCapability::Gesture) != 0 {
                    let device_name = CStr::from_ptr(libinput_device_get_name(libinput_dev)).to_str().unwrap();
                    let product_id = libinput_device_get_id_product(libinput_dev);
                    let vendor_id = libinput_device_get_id_vendor(libinput_dev);
                    return Device::new(device_path, device_name, product_id, vendor_id);
                }

                libinput_path_remove_device(libinput_dev);
            }
        }

        None
    }

    pub fn get_valid_device_list(&mut self) -> Vec<Device> {
        if self.valid_device_list.is_empty() {
            let devices = fs::read_dir(DEVICES_PATH).unwrap();
            for device in devices {
                if let Ok(device) = device {
                    let file_type = device.file_type().unwrap();
                    if file_type.is_char_device() {
                        let device = self.create_device(device.path().to_str().unwrap());
                        if device.is_some() {
                            self.valid_device_list.push(device.unwrap());
                        }
                    }
                }
            }
        }

        self.valid_device_list.to_vec()
    }

    pub fn set_active_device(&mut self, device: &Device) {
        unsafe {
            if !self.active_device.is_null() {
                if (*(self.active_device)).path == device.path { return; }
            }
        }

        let search_result = self.valid_device_list.binary_search_by(|probe| device.path.cmp(&probe.path));
        if search_result.is_err() { return; }

        if self.input.active_device != 0 as *mut libc::c_void {
            let active_device = self.input.active_device;
            self.input.active_device = 0 as *mut libc::c_void;
            unsafe {
                libinput_path_remove_device(active_device);
                libinput_device_unref(active_device);
            }
        }

        unsafe {
            // HACK: For some reason when passing a string into C land it has some junk on the end
            //       so we allocate a new buffer where we copy .len() bytes into and pass it along
            //       to the function that takes a char *
            let mut device_path_vec: Vec<libc::c_char> = Vec::new();
            device_path_vec.resize(device.path.len() + 1, 0);
            let device_path_cstr = strncpy(device_path_vec.as_mut_ptr(), device.path.as_str().as_ptr() as *const libc::c_char, device.path.len());

            let new_device = libinput_path_add_device(self.input.instance, device_path_cstr);
            libinput_device_ref(new_device);
            if new_device as usize != 0 {
                self.input.active_device = new_device;
                self.active_device = &self.valid_device_list[search_result.ok().unwrap()] as *const Device;
            }
        }
    }

    fn handle_swipe_gesture(&mut self, gesture_event: *mut libc::c_void, event_type: libinput::EventType) -> (GestureEventState, i32) {
        let gesture_state;

        let finger_count = unsafe {
            libinput_event_gesture_get_finger_count(gesture_event)
        };

        let mut x_max = self.input.swipe_x_max;
        let mut y_max = self.input.swipe_y_max;

        match event_type {
            libinput::EventType::GestureSwipeBegin => {
                gesture_state = GestureEventState::Started;
            },
            libinput::EventType::GestureSwipeUpdate => {
                gesture_state = GestureEventState::Ongoing;

                let x_current = unsafe { libinput_event_gesture_get_dx_unaccelerated(gesture_event) };
                let y_current = unsafe { libinput_event_gesture_get_dy_unaccelerated(gesture_event) };

                if x_max.abs() < x_current.abs() { x_max = x_current; }
                if y_max.abs() < y_current.abs() { y_max = y_current; }

                if y_max.abs() > x_max.abs() {
                    if y_max < -GESTURE_DELTA {
                        self.ongoing_gesture_type = GestureType::Swipe(SwipeDirection::SwipeUp);
                    } else if y_max > GESTURE_DELTA {
                        self.ongoing_gesture_type = GestureType::Swipe(SwipeDirection::SwipeDown);
                    }
                } else if x_max.abs() > y_max.abs() {
                    if x_max < -GESTURE_DELTA {
                        self.ongoing_gesture_type = GestureType::Swipe(SwipeDirection::SwipeLeft);
                    } else if x_max > GESTURE_DELTA {
                        self.ongoing_gesture_type = GestureType::Swipe(SwipeDirection::SwipeRight);
                    }
                }
            },
            libinput::EventType::GestureSwipeEnd => {
                gesture_state = GestureEventState::Finished;
            }
            _ => { gesture_state = GestureEventState::Unknown; }
        }

        self.input.swipe_x_max = x_max;
        self.input.swipe_y_max = y_max;

        (gesture_state, finger_count)
    }

    fn handle_pinch_gesture(&mut self, gesture_event: *mut libc::c_void, event_type: libinput::EventType) -> (GestureEventState, i32) {
        let gesture_state;

        let finger_count = unsafe {
            libinput_event_gesture_get_finger_count(gesture_event)
        };

        match event_type {
            libinput::EventType::GesturePinchBegin => {
                gesture_state = GestureEventState::Started;
            },
            libinput::EventType::GesturePinchUpdate => {
                gesture_state = GestureEventState::Ongoing;

                let scale = unsafe {
                    libinput_event_gesture_get_scale(gesture_event)
                };

                if scale > 1.0 { self.ongoing_gesture_type = GestureType::Pinch(PinchType::PinchOut); }
                if scale < 1.0 { self.ongoing_gesture_type = GestureType::Pinch(PinchType::PinchIn); }
            },
            libinput::EventType::GesturePinchEnd => {
                gesture_state = GestureEventState::Finished;
            }
            _ => { gesture_state = GestureEventState::Unknown; }
        };

        (gesture_state, finger_count)
    }

    fn handle_gesture(&mut self, event: *mut libc::c_void) {
        let gesture_state;
        let finger_count;
        let event_type = unsafe {
            libinput_event_get_type(event)
        };

        let gesture_event;

        match event_type {
            libinput::EventType::GestureSwipeBegin => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_swipe_gesture(gesture_event, event_type);
                gesture_state = gs;
                finger_count = fc;
            },
            libinput::EventType::GestureSwipeUpdate => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_swipe_gesture(gesture_event, event_type);
                gesture_state = gs;
                finger_count = fc;
            },
            libinput::EventType::GestureSwipeEnd => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_swipe_gesture(gesture_event, event_type);
                gesture_state = gs;
                finger_count = fc;
            },
            libinput::EventType::GesturePinchBegin => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_pinch_gesture(gesture_event, event_type);
                gesture_state= gs;
                finger_count = fc;
            },
            libinput::EventType::GesturePinchUpdate => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_pinch_gesture(gesture_event, event_type);
                gesture_state = gs;
                finger_count = fc;
            },
            libinput::EventType::GesturePinchEnd => {
                gesture_event = unsafe {
                    libinput_event_get_gesture_event(event)
                };

                let (gs, fc) = self.handle_pinch_gesture(gesture_event, event_type);
                gesture_state = gs;
                finger_count = fc;
            },
            _ => {
                gesture_event = 0 as *mut libc::c_void;
                gesture_state = GestureEventState::Unknown;
                finger_count = 0;
            },
        }

        if gesture_state == GestureEventState::Finished {
            unsafe {
                if libinput_event_gesture_get_cancelled(gesture_event) == 0 {
                    match self.ongoing_gesture_type.borrow() {
                        GestureType::Swipe(swipe_direction) => {
                            (self.swipe_delegate)(*swipe_direction, finger_count);
                        },
                        GestureType::Pinch(pinch_type) => {
                            (self.pinch_delegate)(*pinch_type, finger_count);
                        },
                        GestureType::Unknown => {},
                    }
                    self.ongoing_gesture_type = GestureType::Unknown;
                    self.input.swipe_x_max = 0.0;
                    self.input.swipe_y_max = 0.0;
                }
            }
        }
    }

    unsafe extern "C" fn on_event_ready(data: *mut libc::c_void) -> i32 {
        let self_ = &mut *(data as *mut KinesixBackend);

        if self_.event_poller_thread.is_some() {
            let evt_poller = self_.event_poller_thread.as_ref().unwrap();

            if evt_poller.cancelation_requested { return 0; }

            if evt_poller.libinput_event_listener.try_recv().is_ok() {
                loop {
                    let ev = libinput_get_event(self_.input.instance);
                    if ev != 0 as *mut libc::c_void {
                        self_.handle_gesture(ev);
                        libinput_event_destroy(ev);
                    } else {
                        break;
                    }
                }
            }
        }

        1
    }

    pub fn start_polling(&mut self) {
        let (cancel_token_sender, cancel_token_receiver) = mpsc::channel();
        let (libinput_event_listener_sender, libinput_event_listener_receiver) = mpsc::channel();
        let input_fd = unsafe {
            libinput_get_fd(self.input.instance)
        };
        let libinput_handle = self.input.instance as usize;

        self.event_poller_thread = Some(EventPollerThread {
            handle: Some(std::thread::spawn(move || {
                let mut poller = libc::pollfd {
                    fd: input_fd,
                    events: POLLIN,
                    revents: 0,
                };

                loop {
                    let cancelation_requested = cancel_token_receiver.recv_timeout(Duration::new(0, 1000000));
                    if cancelation_requested.is_ok() {
                        if cancelation_requested.unwrap() {
                            println!("done!");
                            break;
                        }
                    }

                    unsafe {
                        /* Wait for an event to be ready by polling the internal libinput fd */
                        poll(&mut poller as *mut libc::pollfd, 1, 500);
                    }

                    if poller.revents == POLLIN {
                        unsafe { libinput_dispatch(libinput_handle as *const libc::c_void); }
                        /* Notify main thread that an event is ready and to add it to the event queue */
                        libinput_event_listener_sender.send(()).expect("Failed to handle event");
                    }
                }
            })),
            cancelation_token: cancel_token_sender,
            cancelation_requested: false,
            libinput_event_listener: libinput_event_listener_receiver,
        });

        unsafe {
            g_timeout_add_full(200, 1, KinesixBackend::on_event_ready, self as *mut KinesixBackend as *mut libc::c_void, 0 as *mut libc::c_void);
        }
    }

    pub fn stop_polling(&mut self) {
        if self.event_poller_thread.is_some() {
            let evt_poller = self.event_poller_thread.as_mut().unwrap();
            evt_poller.cancelation_requested = true;
            evt_poller.cancelation_token.send(true).expect("Failed to set cancelation token for poller thread");
            evt_poller.join();
        }
    }
}

impl Drop for KinesixBackend {
    fn drop(&mut self) {
        self.stop_polling();
    }
}
