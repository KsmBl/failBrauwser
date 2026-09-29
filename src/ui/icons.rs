//! Drawing file icons at any size: GTK 3's cell renderers only know a few fixed icon sizes,
//! so icons are looked up in the theme at the exact pixel size (cached) and drawn as images.

use super::model;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::gio;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static CACHE: RefCell<HashMap<(String, i32), Option<Pixbuf>>> = RefCell::new(HashMap::new());
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

pub fn clear_cache() {
    CACHE.with(|c| c.borrow_mut().clear());
}

/// The theme's image for `icon` at `size` pixels.
pub fn pixbuf_for(icon: &gio::Icon, size: i32) -> Option<Pixbuf> {
    let theme = gtk::IconTheme::default()?;
    if !WATCHING.with(|w| w.replace(true)) {
        // A new icon theme draws everything anew.
        theme.connect_changed(|_| clear_cache());
    }
    let key = (IconExt::to_string(icon).map(|s| s.to_string()).unwrap_or_default(), size);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let pixbuf = theme
        .lookup_by_gicon(icon, size, gtk::IconLookupFlags::FORCE_SIZE)
        .or_else(|| theme.lookup_icon("text-x-generic", size, gtk::IconLookupFlags::FORCE_SIZE))
        .and_then(|info| info.load_icon().ok());
    CACHE.with(|c| c.borrow_mut().insert(key, pixbuf.clone()));
    pixbuf
}

/// What a row shows: its icon at `size`.
pub fn row_image(model: &gtk::TreeModel, iter: &gtk::TreeIter, size: i32) -> Option<Pixbuf> {
    let icon = model.value(iter, model::COL_ICON as i32).get::<Option<gio::Icon>>().ok().flatten()?;
    pixbuf_for(&icon, size)
}

/// Makes `renderer` draw the row image at the size `size` holds.
pub fn bind(layout: &impl IsA<gtk::CellLayout>, renderer: &gtk::CellRendererPixbuf, size: Rc<Cell<i32>>) {
    let r2 = renderer.clone();
    CellLayoutExt::set_cell_data_func(
        layout,
        renderer,
        Some(Box::new(move |_, _, model, iter| {
            let px = size.get();
            r2.set_pixbuf(row_image(model, iter, px).as_ref());
        })),
    );
}
