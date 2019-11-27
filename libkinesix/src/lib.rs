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
use std::future::Future;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::str;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use input::AsRaw;
use libc;

use crate::device::Device;
use std::borrow::Borrow;

const POLLIN: libc::c_short = 0x1;

#[link(name = "mtdev")]
#[link(name = "evdev")]
#[link(name = "wacom")]
#[link(name = "gudev-1.0")]
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
    fn g_timeout_add_full(priority: i32, interval: u32, fun: unsafe extern "C" fn(*mut libc::c_void) -> i32, data: *mut libc::c_void, notify: *mut libc::c_void) -> u32;

    #[no_mangle]
    fn libinput_event_gesture_get_finger_count(gesture_event: *const libc::c_void) -> i32;
}

#[derive(Debug)]
pub struct Input {
    pub instance: input::Libinput,
    pub active_device: Option<input::Device>,

    /* The absolute maximum value for swipe velocity */
    /* These help determine swipe direction */
    pub swipe_x_max: f32,
    pub swipe_y_max: f32,
}

struct LibInputInterface {}

impl input::LibinputInterface for LibInputInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<libc::c_int, i32> {
        let fd;
        let path = path.to_str().unwrap();
        unsafe {
            fd = open(path.as_ptr() as *mut libc::c_char, flags);
            if fd == -1 {
                println!("Failed to open file descriptor.");
                exit(-1);
            }
        }

        Ok(fd)
    }

    fn close_restricted(&mut self, fd: libc::c_int) {
        unsafe {
            close(fd);
        }
    }
}

impl Input {
    pub fn new() -> Input {
        Input {
            instance: input::Libinput::new_from_path(LibInputInterface {}),
            active_device: None,
            swipe_x_max: 0.0,
            swipe_y_max: 0.0,
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

pub enum SwipeDirection
{
    SWIPE_UP,
    SWIPE_DOWN,
    SWIPE_LEFT,
    SWIPE_RIGHT,
}

pub enum PinchType
{
    PINCH_IN,
    PINCH_OUT,
}

#[derive(Debug)]
pub enum GestureEventState
{
    STARTED,
    ONGOING,
    FINISHED,
    UNKNOWN,
}


#[derive(Debug)]
pub enum GestureType
{
    SWIPE,
    PINCH,
    UNKNOWN,
}

const DEVICES_PATH: &str = "/dev/input/";
const GESTURE_DELTA: i32 = 10;

pub struct KinesixBackend
{
    valid_device_list: Vec<Device>,
    active_device: *const Device,

    swipe_delegate: Box<dyn FnMut(SwipeDirection, u32)>,
    pinch_delegate: Box<dyn FnMut(PinchType, u32)>,

    gesture_type: GestureType,
    input: Input,

    event_poller_thread: Option<EventPollerThread>,
}

impl KinesixBackend
{
    pub fn new<SwipeDelegate: 'static + FnMut(SwipeDirection, u32), PinchDelegate: 'static + FnMut(PinchType, u32)>(swipe_delegate: SwipeDelegate, pinch_delegate: PinchDelegate) -> KinesixBackend {
        KinesixBackend {
            active_device: std::ptr::null(),
            valid_device_list: Vec::new(),
            swipe_delegate: Box::new(swipe_delegate),
            pinch_delegate: Box::new(pinch_delegate),
            gesture_type: GestureType::UNKNOWN,
            input: Input::new(),
            event_poller_thread: None,
        }
    }

    fn create_device(&mut self, device_path: &str) -> Option<Device> {
        let libinput_dev = self.input.instance.path_add_device(device_path);
        if libinput_dev.is_some() {
            let libinput_dev = libinput_dev.unwrap();
            if libinput_dev.has_capability(input::DeviceCapability::Gesture) {
                return Device::new(device_path, libinput_dev.name(), libinput_dev.id_product(), libinput_dev.id_vendor());
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

        if self.input.active_device.is_some() {
            let mut active_device = self.input.active_device.take();
            self.input.active_device = None;
            self.input.instance.path_remove_device(active_device.unwrap());
        }

        let new_device = self.input.instance.path_add_device(device.path.as_str());
        if new_device.is_some() {
            self.input.active_device = new_device;
            self.active_device = &self.valid_device_list[search_result.ok().unwrap()] as *const Device;
        }
    }

    fn handle_swipe_gesture(&mut self, event: &input::event::gesture::GestureSwipeEvent) -> (GestureEventState, i32) {
        let finger_count;
        let gesture_state;
        unsafe {
            finger_count = libinput_event_gesture_get_finger_count(event.as_raw() as *const libc::c_void);
        }
        match event {
            input::event::gesture::GestureSwipeEvent::Begin(swipe_begin) => {
                gesture_state = GestureEventState::STARTED;
            },
            input::event::gesture::GestureSwipeEvent::Update(swipe_update) => {
                gesture_state = GestureEventState::ONGOING;
            },
            input::event::gesture::GestureSwipeEvent::End(swipe_end) => {
                gesture_state = GestureEventState::FINISHED;
            }
        };

        (gesture_state, finger_count)
    }

    fn handle_pinch_gesture(&mut self, event: &input::event::gesture::GesturePinchEvent) -> (GestureEventState, i32) {
        let finger_count;
        let gesture_state;
        unsafe {
            finger_count = libinput_event_gesture_get_finger_count(event.as_raw() as *const libc::c_void);
        }
        match event {
            input::event::gesture::GesturePinchEvent::Begin(pinch_begin) => {
                gesture_state = GestureEventState::STARTED;
            },
            input::event::gesture::GesturePinchEvent::Update(pinch_update) => {
                gesture_state = GestureEventState::ONGOING;
            },
            input::event::gesture::GesturePinchEvent::End(pinch_end) => {
                gesture_state = GestureEventState::FINISHED;
            }
        };
        (gesture_state, finger_count)
    }

    fn handle_gesture(&mut self, event: &input::Event) {
        let gesture_state;
        let finger_count;
        let gesture_type;

        if let input::Event::Gesture(gesture_event) = event {
            match gesture_event {
                input::event::GestureEvent::Pinch(pinch_event) => {
                    let (gs, fc) = self.handle_pinch_gesture(pinch_event);
                    gesture_state = gs;
                    finger_count = fc;
                    gesture_type = GestureType::PINCH;
                },
                input::event::GestureEvent::Swipe(swipe_event) => {
                    let (gs, fc) = self.handle_swipe_gesture(swipe_event);
                    gesture_state = gs;
                    finger_count = fc;
                    gesture_type = GestureType::SWIPE;
                }
            }

            println!("{:?}: {:?}: {}", gesture_type, gesture_state, finger_count);
        }

//        if (gesture_state == GestureFinished) && (event.GestureEvent. libinput_event_gesture_get_cancelled(libinput_event_get_gesture_event(event)) == 0))
//        {
//            if ((gesture_type == GestureSwipe) && (self->swiped_cb != 0))
//            self->swiped_cb(self->gesture_type, finger_count, self->swiped_cb_user_data);
//            if ((gesture_type == GesturePinch) && (self->pinch_cb!= 0))
//            self->pinch_cb(self->gesture_type, finger_count, self->pinch_cb_user_data);
//        }
//
//        libinput_event_destroy(event);
    }

    unsafe extern "C" fn on_event_ready(data: *mut libc::c_void) -> i32 {
        let self_ = &mut *(data as *mut KinesixBackend);

        if self_.event_poller_thread.is_some() {
            let evt_poller = self_.event_poller_thread.as_ref().unwrap();

            if evt_poller.cancelation_requested { return 0; }

            if evt_poller.libinput_event_listener.try_recv().is_ok() {
                self_.input.instance.dispatch();
                loop {
                    let ev = self_.input.instance.next();
                    if ev.is_some() {
                        self_.handle_gesture(ev.as_ref().unwrap());
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
        let input_fd = self.input.instance.as_raw_fd();

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
                        /* Notify main thread that an event is ready and to add it to the event queue */
                        libinput_event_listener_sender.send(());

                        /* TODO: Get the actual event from the queue and send it for processing */
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
            evt_poller.cancelation_token.send(true);
            evt_poller.join();
        }
    }
}

impl Drop for KinesixBackend {
    fn drop(&mut self) {
        self.stop_polling();
    }
}

//
//
//extern "C" {
//    #[no_mangle]
//    fn fabs(_: libc::c_double) -> libc::c_double;
//    #[no_mangle]
//    fn __errno_location() -> *mut libc::c_int;
//    #[no_mangle]
//    fn opendir(__name: *const libc::c_char) -> *mut DIR;
//    #[no_mangle]
//    fn readdir(__dirp: *mut DIR) -> *mut dirent;
//    #[no_mangle]
//    fn open(__file: *const libc::c_char, __oflag: libc::c_int, _: ...)
//            -> libc::c_int;
//    #[no_mangle]
//    fn poll(__fds: *mut pollfd, __nfds: nfds_t, __timeout: libc::c_int)
//            -> libc::c_int;
//}
//
//pub type __uint32_t = libc::c_uint;
//pub type __ino64_t = libc::c_ulong;
//pub type __off_t = libc::c_long;
//pub type __off64_t = libc::c_long;
//pub type uint32_t = __uint32_t;
//pub type size_t = libc::c_ulong;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct _IO_FILE {
//    pub _flags: libc::c_int,
//    pub _IO_read_ptr: *mut libc::c_char,
//    pub _IO_read_end: *mut libc::c_char,
//    pub _IO_read_base: *mut libc::c_char,
//    pub _IO_write_base: *mut libc::c_char,
//    pub _IO_write_ptr: *mut libc::c_char,
//    pub _IO_write_end: *mut libc::c_char,
//    pub _IO_buf_base: *mut libc::c_char,
//    pub _IO_buf_end: *mut libc::c_char,
//    pub _IO_save_base: *mut libc::c_char,
//    pub _IO_backup_base: *mut libc::c_char,
//    pub _IO_save_end: *mut libc::c_char,
//    pub _markers: *mut _IO_marker,
//    pub _chain: *mut _IO_FILE,
//    pub _fileno: libc::c_int,
//    pub _flags2: libc::c_int,
//    pub _old_offset: __off_t,
//    pub _cur_column: libc::c_ushort,
//    pub _vtable_offset: libc::c_schar,
//    pub _shortbuf: [libc::c_char; 1],
//    pub _lock: *mut libc::c_void,
//    pub _offset: __off64_t,
//    pub __pad1: *mut libc::c_void,
//    pub __pad2: *mut libc::c_void,
//    pub __pad3: *mut libc::c_void,
//    pub __pad4: *mut libc::c_void,
//    pub __pad5: size_t,
//    pub _mode: libc::c_int,
//    pub _unused2: [libc::c_char; 20],
//}
//pub type _IO_lock_t = ();
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct _IO_marker {
//    pub _next: *mut _IO_marker,
//    pub _sbuf: *mut _IO_FILE,
//    pub _pos: libc::c_int,
//}
//pub type FILE = _IO_FILE;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct __pthread_internal_list {
//    pub __prev: *mut __pthread_internal_list,
//    pub __next: *mut __pthread_internal_list,
//}
//pub type __pthread_list_t = __pthread_internal_list;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct __pthread_mutex_s {
//    pub __lock: libc::c_int,
//    pub __count: libc::c_uint,
//    pub __owner: libc::c_int,
//    pub __nusers: libc::c_uint,
//    pub __kind: libc::c_int,
//    pub __spins: libc::c_short,
//    pub __elision: libc::c_short,
//    pub __list: __pthread_list_t,
//}
//pub type pthread_t = libc::c_ulong;
//#[derive ( Copy, Clone )]
//#[repr ( C )]
//pub union pthread_mutexattr_t {
//    pub __size: [libc::c_char; 4],
//    pub __align: libc::c_int,
//}
//#[derive ( Copy, Clone )]
//#[repr ( C )]
//pub union pthread_attr_t {
//    pub __size: [libc::c_char; 56],
//    pub __align: libc::c_long,
//}
//#[derive ( Copy, Clone )]
//#[repr ( C )]
//pub union pthread_mutex_t {
//    pub __data: __pthread_mutex_s,
//    pub __size: [libc::c_char; 40],
//    pub __align: libc::c_long,
//}
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct _KinesixDevice {
//    pub id: libc::c_int,
//    pub path: *mut libc::c_char,
//    pub name: *mut libc::c_char,
//    pub product_id: uint32_t,
//    pub vendor_id: uint32_t,
//}
//pub type KinesixDevice = _KinesixDevice;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct _KinesixInterface {
//    pub active_device: *mut KinesixDevice,
//    pub valid_device_list: *mut *mut KinesixDevice,
//    pub swiped_cb: SwipedCallback,
//    pub swiped_cb_user_data: *mut libc::c_void,
//    pub pinch_cb: PinchCallback,
//    pub pinch_cb_user_data: *mut libc::c_void,
//    pub gesture_type: libc::c_int,
//    pub libinput: _LibInput,
//    pub event_poller_thread: _EventPollerThread,
//}
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct _EventPollerThread {
//    pub thread_id: pthread_t,
//    pub attr: pthread_attr_t,
//    pub stop_issued: libc::c_int,
//    pub stop_mutex: pthread_mutex_t,
//}
//pub type PinchCallback
//    =
//    Option<unsafe extern "C" fn(_: PinchType, _: libc::c_int,
//                                _: *mut libc::c_void) -> ()>;
//pub type SwipedCallback
//    =
//    Option<unsafe extern "C" fn(_: SwipeDirection, _: libc::c_int,
//                                _: *mut libc::c_void) -> ()>;
//pub type KinesixInterface = _KinesixInterface;
//pub type DIR = __dirstream;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct dirent {
//    pub d_ino: __ino64_t,
//    pub d_off: __off64_t,
//    pub d_reclen: libc::c_ushort,
//    pub d_type: libc::c_uchar,
//    pub d_name: [libc::c_char; 256],
//}
//pub type libinput_device_capability = libc::c_uint;
//pub const LIBINPUT_DEVICE_CAP_SWITCH: libinput_device_capability = 6;
//pub const LIBINPUT_DEVICE_CAP_GESTURE: libinput_device_capability = 5;
//pub const LIBINPUT_DEVICE_CAP_TABLET_PAD: libinput_device_capability = 4;
//pub const LIBINPUT_DEVICE_CAP_TABLET_TOOL: libinput_device_capability = 3;
//pub const LIBINPUT_DEVICE_CAP_TOUCH: libinput_device_capability = 2;
//pub const LIBINPUT_DEVICE_CAP_POINTER: libinput_device_capability = 1;
//pub const LIBINPUT_DEVICE_CAP_KEYBOARD: libinput_device_capability = 0;
//pub const DT_CHR: C2RustUnnamed = 2;
//pub const PTHREAD_CREATE_JOINABLE: C2RustUnnamed_0 = 0;
//pub const GesturePinch: GestureType = 1;
//pub const GestureUnknown: GestureType = 2;
//pub const GestureSwipe: GestureType = 0;
//pub const GestureFinished: GestureEventState = 2;
//pub const GestureStateUnknown: GestureEventState = 3;
//pub const GestureOngoing: GestureEventState = 1;
//pub const GestureStarted: GestureEventState = 0;
//pub const LIBINPUT_EVENT_GESTURE_PINCH_END: libinput_event_type = 805;
//pub type libinput_event_type = libc::c_uint;
//pub const LIBINPUT_EVENT_SWITCH_TOGGLE: libinput_event_type = 900;
//pub const LIBINPUT_EVENT_GESTURE_PINCH_UPDATE: libinput_event_type = 804;
//pub const LIBINPUT_EVENT_GESTURE_PINCH_BEGIN: libinput_event_type = 803;
//pub const LIBINPUT_EVENT_GESTURE_SWIPE_END: libinput_event_type = 802;
//pub const LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE: libinput_event_type = 801;
//pub const LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN: libinput_event_type = 800;
//pub const LIBINPUT_EVENT_TABLET_PAD_STRIP: libinput_event_type = 702;
//pub const LIBINPUT_EVENT_TABLET_PAD_RING: libinput_event_type = 701;
//pub const LIBINPUT_EVENT_TABLET_PAD_BUTTON: libinput_event_type = 700;
//pub const LIBINPUT_EVENT_TABLET_TOOL_BUTTON: libinput_event_type = 603;
//pub const LIBINPUT_EVENT_TABLET_TOOL_TIP: libinput_event_type = 602;
//pub const LIBINPUT_EVENT_TABLET_TOOL_PROXIMITY: libinput_event_type = 601;
//pub const LIBINPUT_EVENT_TABLET_TOOL_AXIS: libinput_event_type = 600;
//pub const LIBINPUT_EVENT_TOUCH_FRAME: libinput_event_type = 504;
//pub const LIBINPUT_EVENT_TOUCH_CANCEL: libinput_event_type = 503;
//pub const LIBINPUT_EVENT_TOUCH_MOTION: libinput_event_type = 502;
//pub const LIBINPUT_EVENT_TOUCH_UP: libinput_event_type = 501;
//pub const LIBINPUT_EVENT_TOUCH_DOWN: libinput_event_type = 500;
//pub const LIBINPUT_EVENT_POINTER_AXIS: libinput_event_type = 403;
//pub const LIBINPUT_EVENT_POINTER_BUTTON: libinput_event_type = 402;
//pub const LIBINPUT_EVENT_POINTER_MOTION_ABSOLUTE: libinput_event_type = 401;
//pub const LIBINPUT_EVENT_POINTER_MOTION: libinput_event_type = 400;
//pub const LIBINPUT_EVENT_KEYBOARD_KEY: libinput_event_type = 300;
//pub const LIBINPUT_EVENT_DEVICE_REMOVED: libinput_event_type = 2;
//pub const LIBINPUT_EVENT_DEVICE_ADDED: libinput_event_type = 1;
//pub const LIBINPUT_EVENT_NONE: libinput_event_type = 0;
//#[derive ( Copy, Clone )]
//#[repr(C)]
//pub struct pollfd {
//    pub fd: libc::c_int,
//    pub events: libc::c_short,
//    pub revents: libc::c_short,
//}
//pub type nfds_t = libc::c_ulong;
//pub type C2RustUnnamed = libc::c_uint;
//pub const DT_WHT: C2RustUnnamed = 14;
//pub const DT_SOCK: C2RustUnnamed = 12;
//pub const DT_LNK: C2RustUnnamed = 10;
//pub const DT_REG: C2RustUnnamed = 8;
//pub const DT_BLK: C2RustUnnamed = 6;
//pub const DT_DIR: C2RustUnnamed = 4;
//pub const DT_FIFO: C2RustUnnamed = 1;
//pub const DT_UNKNOWN: C2RustUnnamed = 0;
//pub type C2RustUnnamed_0 = libc::c_uint;
//pub const PTHREAD_CREATE_DETACHED: C2RustUnnamed_0 = 1;
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_new(mut swipe_cb: SwipedCallback,
//                                        mut swipe_cb_target:
//                                            *mut libc::c_void,
//                                        mut pinch_cb: PinchCallback,
//                                        mut pinch_cb_target:
//                                            *mut libc::c_void)
// -> *mut KinesixInterface {
//    if swipe_cb_target != pinch_cb_target {
//        let mut log: [libc::c_char; 2048] = [0; 2048];
//        snprintf(log.as_mut_ptr(),
//                 (2048 as libc::c_int - 1 as libc::c_int) as libc::c_ulong,
//                 b"Pinch and Swipe callbacks should belong to the same class!!\x00"
//                     as *const u8 as *const libc::c_char);
//        fprintf(stderr,
//                b"kinesixd: FATAL: %s: %s: %d: %s\n\x00" as *const u8 as
//                    *const libc::c_char,
//                b"../src/libkinesix.c\x00" as *const u8 as
//                    *const libc::c_char,
//                (*::std::mem::transmute::<&[u8; 80],
//                                          &[libc::c_char; 80]>(b"KinesixInterface *libkinesix_new(SwipedCallback, void *, PinchCallback, void *)\x00")).as_ptr(),
//                125 as libc::c_int, log.as_mut_ptr());
//        exit(1 as libc::c_int);
//    }
//    let mut self_0: *mut KinesixInterface =
//        malloc(::std::mem::size_of::<_KinesixInterface>() as libc::c_ulong) as
//            *mut KinesixInterface;
//    (*self_0).active_device = 0 as *mut KinesixDevice;
//    (*self_0).valid_device_list = 0 as *mut *mut KinesixDevice;
//    (*self_0).swiped_cb = swipe_cb;
//    (*self_0).swiped_cb_user_data = swipe_cb_target;
//    (*self_0).pinch_cb = pinch_cb;
//    (*self_0).pinch_cb_user_data = pinch_cb_target;
//    (*self_0).gesture_type = -(1 as libc::c_int);
//    (*self_0).libinput.interface.open_restricted =
//        Some(libkinesix_priv_libinput_open_restricted as
//                 unsafe extern "C" fn(_: *const libc::c_char, _: libc::c_int,
//                                      _: *mut libc::c_void) -> libc::c_int);
//    (*self_0).libinput.interface.close_restricted =
//        Some(libkinesix_priv_libinput_close_restricted as
//                 unsafe extern "C" fn(_: libc::c_int, _: *mut libc::c_void)
//                     -> ());
//    (*self_0).libinput.instance =
//        libinput_path_create_context(&mut (*self_0).libinput.interface,
//                                     0 as *mut libc::c_void);
//    (*self_0).libinput.swipe_x_max = 0 as libc::c_int as libc::c_double;
//    (*self_0).libinput.swipe_y_max = 0 as libc::c_int as libc::c_double;
//    pthread_attr_init(&mut (*self_0).event_poller_thread.attr);
//    pthread_attr_setdetachstate(&mut (*self_0).event_poller_thread.attr,
//                                PTHREAD_CREATE_JOINABLE as libc::c_int);
//    pthread_mutex_init(&mut (*self_0).event_poller_thread.stop_mutex,
//                       0 as *const pthread_mutexattr_t);
//    (*self_0).event_poller_thread.stop_issued = 0 as libc::c_int;
//    /* TODO:                                                                                      */
//    /* It might be usefull to set up inotify for /dev/input in order to detect new devices        */
//    /* For now we stick to a static list initialized at the same time as the GestureDeamon itself */
//    (*self_0).valid_device_list = libkinesix_get_valid_device_list(self_0);
//    return self_0;
//}
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_free(mut self_0: *mut KinesixInterface) {
//    libkinesix_stop_polling(self_0);
//    pthread_attr_destroy(&mut (*self_0).event_poller_thread.attr);
//    if !(*self_0).libinput.active_device.is_null() {
//        libinput_path_remove_device((*self_0).libinput.active_device);
//    }
//    libinput_unref((*self_0).libinput.instance);
//    libkinesix_device_list_free((*self_0).valid_device_list);
//    free(self_0 as *mut libc::c_void);
//}
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_get_valid_device_list(mut self_0:
//                                                              *const KinesixInterface)
// -> *mut *mut KinesixDevice {
//    let mut device_list_heap: *mut *mut KinesixDevice =
//        (*self_0).valid_device_list;
//    if (*self_0).valid_device_list.is_null() {
//        let mut device_count: libc::c_int = 0 as libc::c_int;
//        let mut device_list: [*mut KinesixDevice; 255] =
//            [0 as *mut KinesixDevice; 255];
//        let mut device_list_ptr: *mut *mut KinesixDevice =
//            &mut *device_list.as_mut_ptr().offset(0 as libc::c_int as isize)
//                as *mut *mut KinesixDevice;
//        let mut file: *mut dirent = 0 as *mut dirent;
//        let mut dir: *mut DIR = 0 as *mut DIR;
//        dir = opendir(DEVICES_PATH.as_ptr());
//        if !dir.is_null() {
//            loop  {
//                file = readdir(dir);
//                /* Check to see if there are files left to check */
//                if file.is_null() { break ; }
//                /* Check to see if file is a characted device */
//                if (*file).d_type as libc::c_int == DT_CHR as libc::c_int {
//                    libkinesix_priv_add_device(self_0,
//                                               (*file).d_name.as_mut_ptr(),
//                                               &mut device_list_ptr,
//                                               &mut device_count);
//                }
//            }
//        }
//        free(dir as *mut libc::c_void);
//        device_list_heap =
//            libkinesix_priv_device_list_duplicate(device_list.as_mut_ptr(),
//                                                  device_count)
//    }
//    return device_list_heap;
//}
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_set_active_device(mut self_0:
//                                                          *mut KinesixInterface,
//                                                      mut device:
//                                                          *mut KinesixDevice) {
//    if libkinesix_device_equals((*self_0).active_device, device) == 0 {
//        if libkinesix_device_list_contains((*self_0).valid_device_list,
//                                           device) != 0 {
//            if !(*self_0).libinput.active_device.is_null() {
//                libinput_path_remove_device((*self_0).libinput.active_device);
//            }
//            (*self_0).active_device = device;
//            (*self_0).libinput.active_device =
//                libinput_path_add_device((*self_0).libinput.instance,
//                                         libkinesix_device_get_path(device))
//        } else {
//            let mut log: [libc::c_char; 2048] = [0; 2048];
//            snprintf(log.as_mut_ptr(),
//                     (2048 as libc::c_int - 1 as libc::c_int) as
//                         libc::c_ulong,
//                     b"Device %s is not a valid device\x00" as *const u8 as
//                         *const libc::c_char,
//                     libkinesix_device_get_path(device));
//            fprintf(stderr,
//                    b"kinesixd: ERROR: %s: %s: %d: %s\n\x00" as *const u8 as
//                        *const libc::c_char,
//                    b"../src/libkinesix.c\x00" as *const u8 as
//                        *const libc::c_char,
//                    (*::std::mem::transmute::<&[u8; 71],
//                                              &[libc::c_char; 71]>(b"void libkinesix_set_active_device(KinesixInterface *, KinesixDevice *)\x00")).as_ptr(),
//                    225 as libc::c_int, log.as_mut_ptr());
//        }
//    } else {
//        let mut log_0: [libc::c_char; 2048] = [0; 2048];
//        snprintf(log_0.as_mut_ptr(),
//                 (2048 as libc::c_int - 1 as libc::c_int) as libc::c_ulong,
//                 b"Device %s is already active\x00" as *const u8 as
//                     *const libc::c_char, libkinesix_device_get_path(device));
//        fprintf(stderr,
//                b"kinesixd: WARNING: %s: %s: %d: %s\n\x00" as *const u8 as
//                    *const libc::c_char,
//                b"../src/libkinesix.c\x00" as *const u8 as
//                    *const libc::c_char,
//                (*::std::mem::transmute::<&[u8; 71],
//                                          &[libc::c_char; 71]>(b"void libkinesix_set_active_device(KinesixInterface *, KinesixDevice *)\x00")).as_ptr(),
//                230 as libc::c_int, log_0.as_mut_ptr());
//    };
//}
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_start_polling(mut self_0:
//                                                      *mut KinesixInterface) {
//    (*self_0).event_poller_thread.stop_issued = 0 as libc::c_int;
//    pthread_create(&mut (*self_0).event_poller_thread.thread_id,
//                   &mut (*self_0).event_poller_thread.attr,
//                   Some(libkinesix_priv_poll_events as
//                            unsafe extern "C" fn(_: *mut libc::c_void)
//                                -> *mut libc::c_void),
//                   self_0 as *mut libc::c_void);
//}
//#[no_mangle]
//pub unsafe extern "C" fn libkinesix_stop_polling(mut self_0:
//                                                     *mut KinesixInterface) {
//    pthread_mutex_lock(&mut (*self_0).event_poller_thread.stop_mutex);
//    (*self_0).event_poller_thread.stop_issued = 1 as libc::c_int;
//    pthread_mutex_unlock(&mut (*self_0).event_poller_thread.stop_mutex);
//    pthread_join((*self_0).event_poller_thread.thread_id,
//                 0 as *mut *mut libc::c_void);
//}
//unsafe extern "C" fn libkinesix_priv_sanitize_device_name(mut device_name:
//                                                              *const libc::c_char,
//                                                          mut buffer:
//                                                              *mut libc::c_char,
//                                                          mut buffer_size:
//                                                              size_t) {
//    let mut stop: libc::c_int = 0 as libc::c_int;
//    let mut device_name_it: size_t = 0 as libc::c_int as size_t;
//    let mut buffer_it: size_t = 0 as libc::c_int as size_t;
//    let mut undeline_counter: size_t = 0 as libc::c_int as size_t;
//    while stop == 0 {
//        if *device_name.offset(device_name_it as isize) as libc::c_int ==
//               '\u{0}' as i32 ||
//               buffer_it ==
//                   buffer_size.wrapping_sub(1 as libc::c_int as libc::c_ulong)
//           {
//            stop = 1 as libc::c_int
//        }
//        if *device_name.offset(device_name_it as isize) as libc::c_int ==
//               '_' as i32 {
//            if undeline_counter == 0 {
//                undeline_counter = undeline_counter.wrapping_add(1);
//                *buffer.offset(buffer_it as isize) =
//                    ' ' as i32 as libc::c_char
//            } else { buffer_it = buffer_it.wrapping_sub(1) }
//        } else {
//            if undeline_counter != 0 {
//                undeline_counter = 0 as libc::c_int as size_t
//            }
//            *buffer.offset(buffer_it as isize) =
//                *device_name.offset(device_name_it as isize)
//        }
//        device_name_it = device_name_it.wrapping_add(1);
//        buffer_it = buffer_it.wrapping_add(1)
//    }
//    *buffer.offset(buffer_it as isize) = '\u{0}' as i32 as libc::c_char;
//}
//unsafe extern "C" fn libkinesix_priv_add_device(mut self_0:
//                                                    *const KinesixInterface,
//                                                mut device_name:
//                                                    *const libc::c_char,
//                                                mut device_list_out:
//                                                    *mut *mut *mut KinesixDevice,
//                                                mut current_index_out:
//                                                    *mut libc::c_int) {
//    let mut new_device: *mut KinesixDevice = 0 as *mut KinesixDevice;
//    let mut libinput_dev: *mut libinput_device = 0 as *mut libinput_device;
//    let vla =
//        strlen(DEVICES_PATH.as_ptr()).wrapping_add(strlen(device_name)).wrapping_add(1
//                                                                                         as
//                                                                                         libc::c_int
//                                                                                         as
//                                                                                         libc::c_ulong)
//            as usize;
//    let mut device_path: Vec<libc::c_char> = ::std::vec::from_elem(0, vla);
//    let mut udev_dev: *mut udev_device = 0 as *mut udev_device;
//    let mut udev_name: *const libc::c_char = 0 as *const libc::c_char;
//    let mut buffer_size: size_t = 100 as libc::c_int as size_t;
//    let mut udev_dev_sanatized_name: [libc::c_char; 100] = [0; 100];
//    sprintf(device_path.as_mut_ptr(),
//            b"%s%s\x00" as *const u8 as *const libc::c_char,
//            DEVICES_PATH.as_ptr(), device_name);
//    libinput_dev =
//        libinput_path_add_device((*self_0).libinput.instance,
//                                 device_path.as_mut_ptr());
//    if !libinput_dev.is_null() {
//        if libinput_device_has_capability(libinput_dev,
//                                          LIBINPUT_DEVICE_CAP_GESTURE) != 0 {
//            udev_dev = libinput_device_get_udev_device(libinput_dev);
//            if !udev_dev.is_null() {
//                udev_name =
//                    udev_device_get_property_value(udev_dev,
//                                                   b"ID_MODEL\x00" as
//                                                       *const u8 as
//                                                       *const libc::c_char)
//            }
//            if udev_name.is_null() {
//                udev_name = libinput_device_get_name(libinput_dev)
//            } else {
//                libkinesix_priv_sanitize_device_name(udev_name,
//                                                     udev_dev_sanatized_name.as_mut_ptr(),
//                                                     buffer_size);
//                udev_name = udev_dev_sanatized_name.as_mut_ptr()
//            }
//            new_device =
//                libkinesix_device_new(device_path.as_mut_ptr(), udev_name,
//                                      libinput_device_get_id_product(libinput_dev),
//                                      libinput_device_get_id_vendor(libinput_dev));
//            if !new_device.is_null() {
//                let fresh0 = *current_index_out;
//                *current_index_out = *current_index_out + 1;
//                let ref mut fresh1 =
//                    **device_list_out.offset(fresh0 as isize);
//                *fresh1 = new_device
//            }
//        }
//        libinput_path_remove_device(libinput_dev);
//    };
//}
//unsafe extern "C" fn libkinesix_priv_device_list_duplicate(mut device_list:
//                                                               *const *mut KinesixDevice,
//                                                           mut size:
//                                                               libc::c_int)
// -> *mut *mut KinesixDevice {
//    let mut result: *mut *mut KinesixDevice = 0 as *mut *mut KinesixDevice;
//    let mut i: libc::c_int = 0;
//    result =
//        malloc(((size + 1 as libc::c_int) as
//                    libc::c_ulong).wrapping_mul(::std::mem::size_of::<*mut KinesixDevice>()
//                                                    as libc::c_ulong)) as
//            *mut *mut KinesixDevice;
//    i = 0 as libc::c_int;
//    while i < size {
//        let ref mut fresh2 = *result.offset(i as isize);
//        *fresh2 = *device_list.offset(i as isize);
//        i += 1
//    }
//    let ref mut fresh3 = *result.offset(size as isize);
//    *fresh3 = 0 as *mut KinesixDevice;
//    return result;
//}
//unsafe extern "C" fn libkinesix_priv_libinput_open_restricted(mut path:
//                                                                  *const libc::c_char,
//                                                              mut flags:
//                                                                  libc::c_int,
//                                                              mut user_data:
//                                                                  *mut libc::c_void)
// -> libc::c_int {
//    let mut fd: libc::c_int = -(1 as libc::c_int);
//    fd = open(path, flags);
//    if fd == -(1 as libc::c_int) {
//        let mut log: [libc::c_char; 2048] = [0; 2048];
//        snprintf(log.as_mut_ptr(),
//                 (2048 as libc::c_int - 1 as libc::c_int) as libc::c_ulong,
//                 b"Failed to open file descriptor at %s. %s\x00" as *const u8
//                     as *const libc::c_char, path,
//                 strerror(*__errno_location()));
//        fprintf(stderr,
//                b"kinesixd: FATAL: %s: %s: %d: %s\n\x00" as *const u8 as
//                    *const libc::c_char,
//                b"../src/libkinesix.c\x00" as *const u8 as
//                    *const libc::c_char,
//                (*::std::mem::transmute::<&[u8; 72],
//                                          &[libc::c_char; 72]>(b"int libkinesix_priv_libinput_open_restricted(const char *, int, void *)\x00")).as_ptr(),
//                360 as libc::c_int, log.as_mut_ptr());
//        exit(1 as libc::c_int);
//    }
//    return fd;
//}
//unsafe extern "C" fn libkinesix_priv_libinput_close_restricted(mut fd:
//                                                                   libc::c_int,
//                                                               mut user_data:
//                                                                   *mut libc::c_void) {
//    close(fd);
//}
//unsafe extern "C" fn libkinesix_priv_handle_swipe_update(mut self_0:
//                                                             *mut KinesixInterface,
//                                                         mut gesture_event:
//                                                             *mut libinput_event_gesture)
// -> libc::c_int {
//    let mut x_max: libc::c_double = (*self_0).libinput.swipe_x_max;
//    let mut y_max: libc::c_double = (*self_0).libinput.swipe_y_max;
//    let mut x_current: libc::c_double = 0 as libc::c_int as libc::c_double;
//    let mut y_current: libc::c_double = 0 as libc::c_int as libc::c_double;
//    let mut swipe_direction: libc::c_int = -(1 as libc::c_int);
//    if gesture_event.is_null() { return swipe_direction }
//    x_current = libinput_event_gesture_get_dx_unaccelerated(gesture_event);
//    y_current = libinput_event_gesture_get_dy_unaccelerated(gesture_event);
//    x_max = if fabs(x_max) < fabs(x_current) { x_current } else { x_max };
//    y_max = if fabs(y_max) < fabs(y_current) { y_current } else { y_max };
//    if fabs(y_max) > fabs(x_max) {
//        if y_max < -GESTURE_DELTA as libc::c_double {
//            swipe_direction = SWIPE_UP as libc::c_int
//        } else if y_max > GESTURE_DELTA as libc::c_double {
//            swipe_direction = SWIPE_DOWN as libc::c_int
//        }
//    } else if fabs(x_max) > fabs(y_max) {
//        if x_max < -GESTURE_DELTA as libc::c_double {
//            swipe_direction = SWIPE_LEFT as libc::c_int
//        } else if x_max > GESTURE_DELTA as libc::c_double {
//            swipe_direction = SWIPE_RIGHT as libc::c_int
//        }
//    }
//    (*self_0).libinput.swipe_x_max = x_max;
//    (*self_0).libinput.swipe_y_max = y_max;
//    return swipe_direction;
//}
//unsafe extern "C" fn libkinesix_priv_handle_pinch_update(mut self_0:
//                                                             *mut KinesixInterface,
//                                                         mut gesture_event:
//                                                             *mut libinput_event_gesture)
// -> libc::c_int {
//    let mut scale: libc::c_double = 1 as libc::c_int as libc::c_double;
//    let mut pinch_type: libc::c_int = -(1 as libc::c_int);
//    if gesture_event.is_null() { return pinch_type }
//    scale = libinput_event_gesture_get_scale(gesture_event);
//    if scale > 1 as libc::c_int as libc::c_double {
//        pinch_type = PINCH_OUT as libc::c_int
//    } else if scale < 1 as libc::c_int as libc::c_double {
//        pinch_type = PINCH_IN as libc::c_int
//    }
//    return pinch_type;
//}
//unsafe extern "C" fn libkinesix_priv_handle_swipe(mut self_0:
//                                                      *mut KinesixInterface,
//                                                  mut event:
//                                                      *mut libinput_event,
//                                                  mut swipe_finger_count_out:
//                                                      *mut libc::c_int)
// -> GestureEventState {
//    let mut gesture_event: *mut libinput_event_gesture =
//        0 as *mut libinput_event_gesture;
//    let mut gesture_event_type: libinput_event_type = LIBINPUT_EVENT_NONE;
//    let mut swipe_finger_count: libc::c_int = 0 as libc::c_int;
//    let mut state: GestureEventState = GestureStateUnknown;
//    if event.is_null() { return state }
//    gesture_event_type = libinput_event_get_type(event);
//    if gesture_event_type as libc::c_uint ==
//           LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN as libc::c_int as libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        swipe_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        state = GestureStarted
//    } else if gesture_event_type as libc::c_uint ==
//                  LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE as libc::c_int as
//                      libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        swipe_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        (*self_0).gesture_type =
//            libkinesix_priv_handle_swipe_update(self_0, gesture_event);
//        state = GestureOngoing
//    } else if gesture_event_type as libc::c_uint ==
//                  LIBINPUT_EVENT_GESTURE_SWIPE_END as libc::c_int as
//                      libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        swipe_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        state = GestureFinished;
//        (*self_0).libinput.swipe_x_max = 0 as libc::c_int as libc::c_double;
//        (*self_0).libinput.swipe_y_max = 0 as libc::c_int as libc::c_double
//    }
//    *swipe_finger_count_out = swipe_finger_count;
//    return state;
//}
//unsafe extern "C" fn libkinesix_priv_handle_pinch(mut self_0:
//                                                      *mut KinesixInterface,
//                                                  mut event:
//                                                      *mut libinput_event,
//                                                  mut pinch_finger_count_out:
//                                                      *mut libc::c_int)
// -> GestureEventState {
//    let mut gesture_event: *mut libinput_event_gesture =
//        0 as *mut libinput_event_gesture;
//    let mut gesture_event_type: libinput_event_type = LIBINPUT_EVENT_NONE;
//    let mut pinch_finger_count: libc::c_int = 0 as libc::c_int;
//    let mut state: GestureEventState = GestureStateUnknown;
//    if event.is_null() { return state }
//    gesture_event_type = libinput_event_get_type(event);
//    if gesture_event_type as libc::c_uint ==
//           LIBINPUT_EVENT_GESTURE_PINCH_BEGIN as libc::c_int as libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        pinch_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        state = GestureStarted
//    } else if gesture_event_type as libc::c_uint ==
//                  LIBINPUT_EVENT_GESTURE_PINCH_UPDATE as libc::c_int as
//                      libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        pinch_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        (*self_0).gesture_type =
//            libkinesix_priv_handle_pinch_update(self_0, gesture_event);
//        state = GestureOngoing
//    } else if gesture_event_type as libc::c_uint ==
//                  LIBINPUT_EVENT_GESTURE_PINCH_END as libc::c_int as
//                      libc::c_uint {
//        gesture_event = libinput_event_get_gesture_event(event);
//        pinch_finger_count =
//            libinput_event_gesture_get_finger_count(gesture_event);
//        state = GestureFinished;
//        (*self_0).libinput.swipe_x_max = 0 as libc::c_int as libc::c_double;
//        (*self_0).libinput.swipe_y_max = 0 as libc::c_int as libc::c_double
//    }
//    *pinch_finger_count_out = pinch_finger_count;
//    return state;
//}
//unsafe extern "C" fn libkinesix_priv_handle_gesture(mut self_0:
//                                                        *mut KinesixInterface,
//                                                    mut event:
//                                                        *mut libinput_event) {
//    let mut finger_count: libc::c_int = 0 as libc::c_int;
//    let mut gesture_type: GestureType = GestureUnknown;
//    let mut gesture_state: GestureEventState = GestureStateUnknown;
//    gesture_state =
//        libkinesix_priv_handle_swipe(self_0, event, &mut finger_count);
//    if gesture_state as libc::c_uint !=
//           GestureStateUnknown as libc::c_int as libc::c_uint {
//        gesture_type = GestureSwipe
//    } else {
//        gesture_state =
//            libkinesix_priv_handle_pinch(self_0, event, &mut finger_count);
//        if gesture_state as libc::c_uint !=
//               GestureStateUnknown as libc::c_int as libc::c_uint {
//            gesture_type = GesturePinch
//        }
//    }
//    if gesture_state as libc::c_uint ==
//           GestureFinished as libc::c_int as libc::c_uint &&
//           libinput_event_gesture_get_cancelled(libinput_event_get_gesture_event(event))
//               == 0 as libc::c_int {
//        if gesture_type as libc::c_uint ==
//               GestureSwipe as libc::c_int as libc::c_uint &&
//               (*self_0).swiped_cb.is_some() {
//            (*self_0).swiped_cb.expect("non-null function pointer")((*self_0).gesture_type
//                                                                        as
//                                                                        SwipeDirection,
//                                                                    finger_count,
//                                                                    (*self_0).swiped_cb_user_data);
//        }
//        if gesture_type as libc::c_uint ==
//               GesturePinch as libc::c_int as libc::c_uint &&
//               (*self_0).pinch_cb.is_some() {
//            (*self_0).pinch_cb.expect("non-null function pointer")((*self_0).gesture_type
//                                                                       as
//                                                                       PinchType,
//                                                                   finger_count,
//                                                                   (*self_0).pinch_cb_user_data);
//        }
//    }
//    libinput_event_destroy(event);
//}
//unsafe extern "C" fn libkinesix_priv_poll_events(mut libkinesix:
//                                                     *mut libc::c_void)
// -> *mut libc::c_void {
//    let mut self_0: *mut KinesixInterface =
//        libkinesix as *mut KinesixInterface;
//    let mut stop_issued: libc::c_int = 0 as libc::c_int;
//    let mut poller: pollfd =
//        {
//            let mut init =
//                pollfd{fd: libinput_get_fd((*self_0).libinput.instance),
//                       events: 0x1 as libc::c_int as libc::c_short,
//                       revents: 0 as libc::c_int as libc::c_short,};
//            init
//        };
//    loop  {
//        pthread_mutex_lock(&mut (*self_0).event_poller_thread.stop_mutex);
//        stop_issued = (*self_0).event_poller_thread.stop_issued;
//        pthread_mutex_unlock(&mut (*self_0).event_poller_thread.stop_mutex);
//        if stop_issued != 0 { break ; }
//        /* Wait for an event to be ready by polling the internal libinput fd */
//        poll(&mut poller, 1 as libc::c_int as nfds_t, 500 as libc::c_int);
//        if poller.revents as libc::c_int == 0x1 as libc::c_int {
//            /* Notify libinput that an event is ready and to add it (hopefully) to the event queue */
//            libinput_dispatch((*self_0).libinput.instance);
//            /* Get the actual event from the queue and send it for processing*/
//            libkinesix_priv_handle_gesture(self_0,
//                                           libinput_get_event((*self_0).libinput.instance));
//        }
//    }
//    pthread_exit(0 as *mut libc::c_void);
//}

