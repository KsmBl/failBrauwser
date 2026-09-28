mod ui;

const HELP: &str = "\
Usage: failbrauwser [OPTION…] [FOLDER|ARCHIVE…]

Opens a window for each location (or your home folder). A running failBrauwser
opens the window instead of starting again.

Options:
  -d, --daemon    start in the background without a window (for autostart)
  -q, --quit      quit the running instance, including a background one
  -h, --help      show this help
  -V, --version   show the version
";

fn main() -> gtk::glib::ExitCode {
    // Answered without GTK or D-Bus: never opens a window.
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return gtk::glib::ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("failbrauwser {}", env!("CARGO_PKG_VERSION"));
                return gtk::glib::ExitCode::SUCCESS;
            }
            _ => {}
        }
    }
    ui::app::run()
}
