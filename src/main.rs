extern crate gtk;
extern crate gio;

use gtk::prelude::*;
use gio::prelude::*;

use gtk::{Application, ApplicationWindow, Button};

use libkinesix;
use std::borrow::Borrow;

fn swipe(dir: libkinesix::SwipeDirection, finger_count: u32) {
}

fn pinch(t: libkinesix::PinchType, finger_count: u32) {
}


fn main() {
    let mut b = libkinesix::KinesixBackend::new(swipe, pinch);
    let devices = b.get_valid_device_list();
    println!("{:?}", devices);
    b.set_active_device(&devices[0]);
    b.start_polling();

    let application = Application::new(
        Some("com.github.gtk-rs.examples.basic"),
        Default::default(),
    ).expect("failed to initialize GTK application");

    application.connect_activate(|app| {
        let window = ApplicationWindow::new(app);
        window.set_title("First GTK+ Program");
        window.set_default_size(350, 70);

        let button = Button::new_with_label("Click me!");
        button.connect_clicked(|_| {
            println!("Clicked!");
        });
        window.add(&button);

        window.show_all();
    });

    application.run(&[]);
}


