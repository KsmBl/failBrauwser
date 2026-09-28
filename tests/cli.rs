//! Command line flags that must work without a display and never open a window.

use std::process::Command;

fn run(arg: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_failbrauwser"))
        .arg(arg)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
        .output()
        .unwrap()
}

#[test]
fn version_and_help_need_no_display() {
    let v = run("--version");
    assert!(v.status.success());
    assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), format!("failbrauwser {}", env!("CARGO_PKG_VERSION")));
    let h = run("--help");
    assert!(h.status.success());
    assert!(String::from_utf8_lossy(&h.stdout).contains("--daemon"));
}
