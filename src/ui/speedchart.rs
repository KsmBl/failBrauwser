//! The speed chart of a copy in the progress window: across, how far the copy has come;
//! up, how fast it went there, with a line and the figure for the speed now. Drawn in the
//! colors tileWin draws its own charts in (green on black and sunken on Windows 95), else in
//! the GTK theme's.

use failbrauwser::fs::format::human_size;
use failbrauwser::wmtheme::{ChartStyle, Rgba};
use gtk::cairo;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// What the chart shows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Samples {
    /// (share done 0..=1, bytes per second) in the order measured.
    pub points: Vec<(f64, f64)>,
    /// Share done now.
    pub done: f64,
    /// Bytes per second now.
    pub speed: f64,
    /// Top of the scale: only grows, so the curve does not jump around.
    pub top: f64,
}

impl Samples {
    /// Adds a measurement; points closer than a pixel or so replace the one before.
    pub fn push(&mut self, done: f64, speed: f64) {
        let done = done.clamp(0.0, 1.0);
        let speed = speed.max(0.0);
        // The first measurement comes a moment in: the curve starts at the left edge.
        if self.points.is_empty() && done > 0.0 {
            self.points.push((0.0, speed));
        }
        if let Some(last) = self.points.last_mut() {
            if done < last.0 {
                return;
            }
            if done - last.0 < 0.002 {
                *last = (last.0, speed);
                self.done = done;
                self.speed = speed;
                self.top = self.top.max(nice_ceiling(speed * 1.15));
                return;
            }
        }
        self.points.push((done, speed));
        self.done = done;
        self.speed = speed;
        self.top = self.top.max(nice_ceiling(speed * 1.15));
    }
}

/// The next round number at or above `v`: 1, 2 or 5 times a power of ten.
pub fn nice_ceiling(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 0.0;
    }
    let p = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|c| *c >= v * 0.999_999).unwrap_or(10.0 * p)
}

pub struct SpeedChart {
    pub area: gtk::DrawingArea,
    pub samples: Rc<RefCell<Samples>>,
}

impl SpeedChart {
    pub fn new(style: Rc<RefCell<Option<ChartStyle>>>) -> SpeedChart {
        let area = gtk::DrawingArea::new();
        area.set_size_request(-1, 76);
        area.set_hexpand(true);
        let samples = Rc::new(RefCell::new(Samples::default()));
        // The figure is drawn by GTK, in the text color the chart asks for.
        let text_css = gtk::CssProvider::new();
        area.style_context().add_provider(&text_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        let applied = std::cell::Cell::new(None::<(u8, u8, u8, u8)>);
        let s = samples.clone();
        area.connect_draw(move |a, cr| {
            let style = style.borrow().clone().unwrap_or_else(|| gtk_style(a));
            let t = style.text;
            let key = Some(((t.0 * 255.0) as u8, (t.1 * 255.0) as u8, (t.2 * 255.0) as u8, (t.3 * 255.0) as u8));
            if applied.replace(key) != key {
                let css = format!("* {{ color: rgba({}, {}, {}, {:.3}); }}", (t.0 * 255.0).round(), (t.1 * 255.0).round(), (t.2 * 255.0).round(), t.3);
                let _ = text_css.load_from_data(css.as_bytes());
            }
            draw(a, cr, &style, &s.borrow());
            gtk::glib::Propagation::Stop
        });
        SpeedChart { area, samples }
    }

    pub fn push(&self, done: f64, speed: f64) {
        self.samples.borrow_mut().push(done, speed);
        self.area.queue_draw();
    }
}

fn gdk_rgba(c: gtk::gdk::RGBA) -> Rgba {
    Rgba(c.red(), c.green(), c.blue(), c.alpha())
}

/// The window's background color (what a chart is drawn on).
pub fn window_bg(w: &impl IsA<gtk::Widget>) -> Rgba {
    let sc = w.style_context();
    sc.lookup_color("theme_bg_color").map(gdk_rgba).unwrap_or(Rgba(0.95, 0.95, 0.95, 1.0))
}

/// A chart in the GTK theme's colors.
fn gtk_style(w: &gtk::DrawingArea) -> ChartStyle {
    let sc = w.style_context();
    let get = |name: &str, d: Rgba| sc.lookup_color(name).map(gdk_rgba).unwrap_or(d);
    ChartStyle::plain(
        get("theme_base_color", Rgba(1.0, 1.0, 1.0, 1.0)),
        get("theme_selected_bg_color", Rgba::from_u32(0x3584e4ff)),
        get("borders", Rgba(0.5, 0.5, 0.5, 1.0)),
        window_bg(w),
    )
}

fn set(cr: &cairo::Context, c: Rgba) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    use std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn pixel_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, c: Rgba) {
    set(cr, c);
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();
}

fn draw(widget: &gtk::DrawingArea, cr: &cairo::Context, st: &ChartStyle, s: &Samples) {
    let (w, h) = (widget.allocated_width() as f64, widget.allocated_height() as f64);
    // Inside the sunken edges of the classic look.
    let inset = if st.bevel.is_some() { 2.0 } else { 0.0 };
    let (x0, y0, cw, ch) = (inset, inset, w - 2.0 * inset, h - 2.0 * inset);
    cr.save().ok();
    rounded(cr, x0, y0, cw, ch, st.radius);
    set(cr, st.bg);
    let _ = cr.fill_preserve();
    cr.clip();
    for i in 1..4 {
        pixel_rect(cr, x0, (y0 + ch * i as f64 / 4.0).floor(), cw, 1.0, st.grid);
    }
    // How far it has come.
    pixel_rect(cr, x0, y0, (cw * s.done).round(), ch, st.line.with_alpha(st.line.3 * 0.10));
    let y_of = |speed: f64| if s.top > 0.0 { y0 + ch - ch * (speed / s.top).min(1.0) } else { y0 + ch };
    if s.points.len() > 1 || (s.points.len() == 1 && s.done > 0.0) {
        cr.move_to(x0 + cw * s.points[0].0, y0 + ch);
        for (d, v) in &s.points {
            cr.line_to(x0 + cw * d, y_of(*v));
        }
        cr.line_to(x0 + cw * s.done, y_of(s.speed));
        cr.line_to(x0 + cw * s.done, y0 + ch);
        cr.close_path();
        set(cr, st.line.with_alpha(st.line.3 * 0x40 as f64 / 255.0));
        let _ = cr.fill();
        cr.move_to(x0 + cw * s.points[0].0, y_of(s.points[0].1));
        for (d, v) in &s.points {
            cr.line_to(x0 + cw * d, y_of(*v));
        }
        cr.line_to(x0 + cw * s.done, y_of(s.speed));
        set(cr, st.line);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    }
    if s.speed > 0.0 {
        // The speed now, across the whole chart, and its figure.
        let y = y_of(s.speed).round() + 0.5;
        cr.move_to(x0, y);
        cr.line_to(x0 + cw, y);
        set(cr, st.text.with_alpha(0.55));
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let layout = widget.create_pango_layout(Some(&format!("Speed: {}/s", human_size(s.speed as u64))));
        let (tw, th) = layout.pixel_size();
        let (tw, th) = (tw as f64, th as f64);
        let ty = if y - th - 2.0 >= y0 + 1.0 { y - th - 2.0 } else { y + 3.0 };
        gtk::render_layout(&widget.style_context(), cr, x0 + cw - tw - 6.0, ty, &layout);
    }
    cr.restore().ok();
    if let Some([hi, light, shadow, dark]) = st.bevel {
        // Sunken: dark above and left, light below and right.
        pixel_rect(cr, 0.0, 0.0, w - 1.0, 1.0, shadow);
        pixel_rect(cr, 0.0, 0.0, 1.0, h - 1.0, shadow);
        pixel_rect(cr, w - 1.0, 0.0, 1.0, h, hi);
        pixel_rect(cr, 0.0, h - 1.0, w, 1.0, hi);
        pixel_rect(cr, 1.0, 1.0, w - 3.0, 1.0, dark);
        pixel_rect(cr, 1.0, 1.0, 1.0, h - 3.0, dark);
        pixel_rect(cr, w - 2.0, 1.0, 1.0, h - 2.0, light);
        pixel_rect(cr, 1.0, h - 2.0, w - 2.0, 1.0, light);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_scale_tops() {
        assert_eq!(nice_ceiling(0.0), 0.0);
        assert_eq!(nice_ceiling(1.0), 1.0);
        assert_eq!(nice_ceiling(1.3), 2.0);
        assert_eq!(nice_ceiling(3.0), 5.0);
        assert_eq!(nice_ceiling(7.0), 10.0);
        assert_eq!(nice_ceiling(115e6), 200e6);
        assert_eq!(nice_ceiling(f64::NAN), 0.0);
    }

    #[test]
    fn samples_follow_the_copy() {
        let mut s = Samples::default();
        s.push(0.0, 0.0);
        s.push(0.1, 100e6);
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.top, 200e6);
        // Hardly moved: the last point is updated instead of a new one.
        s.push(0.1005, 120e6);
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[1].1, 120e6);
        // The scale only grows.
        s.push(0.5, 10e6);
        assert_eq!(s.top, 200e6);
        assert_eq!((s.done, s.speed), (0.5, 10e6));
        // A chart whose first measurement comes in later still starts at the left.
        let mut late = Samples::default();
        late.push(0.05, 30e6);
        assert_eq!(late.points, [(0.0, 30e6), (0.05, 30e6)]);
        // Going back (a retried file) is not drawn.
        s.push(0.4, 50e6);
        assert_eq!(s.points.len(), 3);
        s.push(2.0, -1.0);
        assert_eq!(s.points.last(), Some(&(1.0, 0.0)));
    }
}
