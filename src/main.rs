use gtk::prelude::*;

const APP_ID: &str = "org.failbrauwser.FailBrauwser";

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::new(Some(APP_ID), gtk::gio::ApplicationFlags::HANDLES_OPEN);
    app.connect_activate(|app| {
        let win = gtk::ApplicationWindow::new(app);
        win.set_title("failBrauwser");
        win.set_default_size(900, 600);
        win.show_all();
    });
    app.run()
}
