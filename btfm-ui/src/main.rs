mod application;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

use application::Application;

const APP_ID: &str = "org.jcline.btfm";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> glib::ExitCode {
    gio::resources_register_include!("compiled.gresource")
        .expect("Failed to register GTK resources");
    let app = Application::new(APP_ID, &gio::ApplicationFlags::empty());

    app.run()
}
