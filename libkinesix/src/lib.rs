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
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::str;
use std::sync::mpsc;
use std::time::Duration;

use input::AsRaw;
use input::event::gesture::GestureEventCoordinates;

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
    fn libinput_dispatch(handle: *const libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_event_gesture_get_finger_count(gesture_event: *const libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_event_gesture_get_cancelled(gesture_event: *const libc::c_void) -> i32;

    #[no_mangle]
    fn libinput_event_gesture_get_scale(gesture_event: *const libc::c_void) -> f64;
}

#[derive(Debug)]
pub struct Input {
    pub instance: input::Libinput,
    pub active_device: Option<input::Device>,

    /* The absolute maximum value for swipe velocity */
    /* These help determine swipe direction */
    pub swipe_x_max: f64,
    pub swipe_y_max: f64,
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
    valid_device_list: Vec<Device>,
    active_device: *const Device,

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
            let active_device = self.input.active_device.take();
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
        let gesture_state;

        let finger_count = unsafe {
            libinput_event_gesture_get_finger_count(event.as_raw() as *const libc::c_void)
        };

        let mut x_max = self.input.swipe_x_max;
        let mut y_max = self.input.swipe_y_max;

        match event {
            input::event::gesture::GestureSwipeEvent::Begin(_swipe_begin) => {
                gesture_state = GestureEventState::Started;
            },
            input::event::gesture::GestureSwipeEvent::Update(swipe_update) => {
                gesture_state = GestureEventState::Ongoing;

                let x_current = swipe_update.dx_unaccelerated();
                let y_current = swipe_update.dy_unaccelerated();

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
            input::event::gesture::GestureSwipeEvent::End(_swipe_end) => {
                gesture_state = GestureEventState::Finished;
            }
        };

        self.input.swipe_x_max = x_max;
        self.input.swipe_y_max = y_max;

        (gesture_state, finger_count)
    }

    fn handle_pinch_gesture(&mut self, event: &input::event::gesture::GesturePinchEvent) -> (GestureEventState, i32) {
        let gesture_state;

        let finger_count = unsafe {
            libinput_event_gesture_get_finger_count(event.as_raw() as *const libc::c_void)
        };

        match event {
            input::event::gesture::GesturePinchEvent::Begin(_pinch_begin) => {
                gesture_state = GestureEventState::Started;
            },
            input::event::gesture::GesturePinchEvent::Update(_pinch_update) => {
                gesture_state = GestureEventState::Ongoing;

                let scale = unsafe {
                    libinput_event_gesture_get_scale(event.as_raw() as *const libc::c_void)
                };

                if scale > 1.0 { self.ongoing_gesture_type = GestureType::Pinch(PinchType::PinchOut ); }
                if scale < 1.0 { self.ongoing_gesture_type = GestureType::Pinch(PinchType::PinchIn ); }
            },
            input::event::gesture::GesturePinchEvent::End(_pinch_end) => {
                gesture_state = GestureEventState::Finished;
            }
        };

        (gesture_state, finger_count)
    }

    fn handle_gesture(&mut self, event: &input::Event) {
        let gesture_state;
        let finger_count;

        if let input::Event::Gesture(gesture_event) = event {
            match gesture_event {
                input::event::GestureEvent::Pinch(pinch_event) => {
                    let (gs, fc) = self.handle_pinch_gesture(pinch_event);
                    gesture_state = gs;
                    finger_count = fc;
                },
                input::event::GestureEvent::Swipe(swipe_event) => {
                    let (gs, fc) = self.handle_swipe_gesture(swipe_event);
                    gesture_state = gs;
                    finger_count = fc;
                }
            }

            if gesture_state == GestureEventState::Finished {
                unsafe {
                    if libinput_event_gesture_get_cancelled(gesture_event.as_raw() as *const libc::c_void) == 0 {
                        match self.ongoing_gesture_type.borrow() {
                            GestureType::Swipe(swipe_direction) => {
                                (self.swipe_delegate)(*swipe_direction, finger_count);
                            },
                            GestureType::Pinch(pinch_type) => {
                                (self.pinch_delegate)(*pinch_type, finger_count);
                            },
                            GestureType::Unknown => { },
                        }
                        self.ongoing_gesture_type = GestureType::Unknown;
                        self.input.swipe_x_max = 0.0;
                        self.input.swipe_y_max = 0.0;
                    }
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
        let libinput_handle = self.input.instance.as_raw() as *const libc::c_void as usize;

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
