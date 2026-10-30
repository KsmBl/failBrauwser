//! The file chooser for other applications: an `org.freedesktop.impl.portal.FileChooser`
//! backend on the session bus. xdg-desktop-portal hands it the open and save dialogs of
//! applications that ask the portal (Flatpaks, Firefox, Qt and GTK applications set to use
//! it) when `portals.conf` names failBrauwser for that interface; `install.sh` sets that up.

use super::app::AppCtx;
use gtk::gio;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.failbrauwser";
const OBJECT: &str = "/org/freedesktop/portal/desktop";

const XML: &str = r#"<node>
  <interface name="org.freedesktop.impl.portal.FileChooser">
    <method name="OpenFile">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFile">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFiles">
      <arg type="o" name="handle" direction="in"/><arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/><arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/><arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.impl.portal.Request">
    <method name="Close"/>
  </interface>
</node>"#;

thread_local! {
    static OWNER: RefCell<Option<gio::OwnerId>> = const { RefCell::new(None) };
}

/// Starts answering as the file chooser (once per process).
pub fn serve(app: &Rc<AppCtx>) {
    OWNER.with(|o| {
        if o.borrow().is_some() {
            return;
        }
        let app = Rc::downgrade(app);
        let id = gio::bus_own_name(
            gio::BusType::Session,
            BUS_NAME,
            gio::BusNameOwnerFlags::NONE,
            move |conn, _| register(&conn, app.clone()),
            |_, _| {},
            |_, _| {},
        );
        *o.borrow_mut() = Some(id);
    });
}

fn register(conn: &gio::DBusConnection, app: Weak<AppCtx>) {
    let Ok(node) = gio::DBusNodeInfo::for_xml(XML) else { return };
    let Some(iface) = node.lookup_interface("org.freedesktop.impl.portal.FileChooser") else { return };
    let request_iface = node.lookup_interface("org.freedesktop.impl.portal.Request");
    let conn2 = conn.clone();
    let _ = conn
        .register_object(OBJECT, &iface)
        .method_call(move |_, _, _, _, method, params, invocation| {
            let Some(app) = app.upgrade() else {
                invocation.return_value(Some(&failbrauwser::chooser::reply(None)));
                return;
            };
            let handle: String = params.child_value(0).str().unwrap_or_default().to_string();
            let title: String = params.child_value(3).get().unwrap_or_default();
            let request = failbrauwser::chooser::parse(method, &title, &params.child_value(4));
            // The application may close the dialog itself, through the request's object.
            let registration: Rc<RefCell<Option<gio::RegistrationId>>> = Rc::new(RefCell::new(None));
            let r2 = registration.clone();
            let conn3 = conn2.clone();
            let chooser = super::chooser::open(
                &app,
                request,
                Box::new(move |answer| {
                    if let Some(id) = r2.borrow_mut().take() {
                        let _ = conn3.unregister_object(id);
                    }
                    let picked = answer.as_ref().map(|(paths, filter)| (paths.as_slice(), filter.as_ref()));
                    invocation.return_value(Some(&failbrauwser::chooser::reply(picked)));
                }),
            );
            if let (Some(req), false) = (&request_iface, handle.is_empty()) {
                let weak = Rc::downgrade(&chooser);
                if let Ok(id) = conn2
                    .register_object(&handle, req)
                    .method_call(move |_, _, _, _, _, _, inv| {
                        if let Some(c) = weak.upgrade() {
                            c.cancel();
                        }
                        inv.return_value(None);
                    })
                    .build()
                {
                    *registration.borrow_mut() = Some(id);
                }
            }
        })
        .build();
}
