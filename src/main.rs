extern crate gtk;
extern crate gio;

use gtk::prelude::*;
use gio::prelude::*;
use gio::ApplicationFlags;

use gtk::*;

use kinesix;

fn swipe(dir: kinesix::SwipeDirection, finger_count: i32) {
    println!("SWIPE: {:?}, {} fingers", dir, finger_count)
}

fn pinch(t: kinesix::PinchType, finger_count: i32) {
    println!("PINCH: {:?}, {} fingers", t, finger_count)
}

fn main() {
    let mut b = kinesix::KinesixBackend::new(swipe, pinch);
    let devices = b.get_valid_device_list();
    println!("{:?}", devices);
    b.set_active_device(&devices[0]);
    b.start_polling();

    let application = Application::new(
        Some("com.github.kicsyromy.kinesix"),
        ApplicationFlags::empty()
    ).expect("Failed to create application instance");

    let mut main_window = Window::new(WindowType::Toplevel);

    let device_chooser = ComboBox::new();

    let header = HeaderBar::new();
    header.set_title(Some("Kinesix"));
//    header.set_subtitle(Some("<Selected device goes here>"));
//    header.set_has_subtitle(true);
    header.set_show_close_button(true);
    header.pack_end(&device_chooser);

    main_window.set_titlebar(Some(&header));

    let window_ptr = &main_window as *const Window as *const ::std::os::raw::c_void as usize;
    application.connect_activate(move |app| {
        let window = unsafe { &(*(window_ptr as *mut Window)) };
        app.add_window(window);
    });

    main_window.show_all();
//    application.hold();
    application.run(&[]);
}


