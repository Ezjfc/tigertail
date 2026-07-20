//! Screenshot-style region selection: one transparent layer-shell overlay per
//! monitor, drag to select, Esc to cancel. Returns the selection in global
//! logical coordinates (GDK monitor geometry on Wayland comes from xdg-output,
//! so these agree with the injector's coordinate space).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config::Region;

/// (x0, y0, x1, y1) of an in-progress drag, window-local.
type DragRect = Rc<RefCell<Option<(f64, f64, f64, f64)>>>;

pub fn select(app: &gtk::Application, done: impl Fn(Option<Region>) + 'static) {
    let display = gdk::Display::default().expect("no display");
    let monitors = display.monitors();

    let windows: Rc<RefCell<Vec<gtk::ApplicationWindow>>> = Rc::default();
    let finished = Rc::new(Cell::new(false));
    let done = Rc::new(done);
    let finish: Rc<dyn Fn(Option<Region>)> = {
        let windows = windows.clone();
        let finished = finished.clone();
        Rc::new(move |result| {
            if finished.replace(true) {
                return;
            }
            for w in windows.borrow().iter() {
                w.close();
            }
            done(result);
        })
    };

    for i in 0..monitors.n_items() {
        let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else {
            continue;
        };
        let geo = monitor.geometry();

        let win = gtk::ApplicationWindow::new(app);
        win.init_layer_shell();
        win.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            win.set_anchor(edge, true);
        }
        win.set_exclusive_zone(-1);
        win.set_keyboard_mode(KeyboardMode::Exclusive);
        win.set_monitor(Some(&monitor));
        win.add_css_class("chiral-overlay");

        let area = gtk::DrawingArea::new();
        area.set_cursor_from_name(Some("crosshair"));

        let sel: DragRect = Rc::default();

        {
            let sel = sel.clone();
            area.set_draw_func(move |_, cr, w, _h| {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
                let _ = cr.paint();
                if let Some((x0, y0, x1, y1)) = *sel.borrow() {
                    let (rx, ry) = (x0.min(x1), y0.min(y1));
                    let (rw, rh) = ((x1 - x0).abs(), (y1 - y0).abs());
                    cr.set_operator(gtk::cairo::Operator::Clear);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.fill();
                    cr.set_operator(gtk::cairo::Operator::Over);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                    cr.set_line_width(2.0);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();
                    cr.set_font_size(14.0);
                    let label = format!("{} × {}", rw.round() as i32, rh.round() as i32);
                    let tx = rx.min(w as f64 - 90.0).max(4.0);
                    let ty = (ry - 8.0).max(18.0);
                    cr.move_to(tx, ty);
                    let _ = cr.show_text(&label);
                }
            });
        }

        let drag = gtk::GestureDrag::new();
        {
            let sel = sel.clone();
            let area = area.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                if let Some((sx, sy)) = g.start_point() {
                    *sel.borrow_mut() = Some((sx, sy, sx + dx, sy + dy));
                    area.queue_draw();
                }
            });
        }
        {
            let finish = finish.clone();
            drag.connect_drag_end(move |g, dx, dy| {
                let Some((sx, sy)) = g.start_point() else {
                    return;
                };
                let (x0, y0, x1, y1) = (sx, sy, sx + dx, sy + dy);
                let (rw, rh) = ((x1 - x0).abs(), (y1 - y0).abs());
                if rw < 5.0 || rh < 5.0 {
                    finish(None); // a stray click — treat as cancel
                    return;
                }
                finish(Some(Region {
                    x: geo.x() as f64 + x0.min(x1),
                    y: geo.y() as f64 + y0.min(y1),
                    w: rw,
                    h: rh,
                }));
            });
        }
        area.add_controller(drag);

        let keys = gtk::EventControllerKey::new();
        {
            let finish = finish.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    finish(None);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        win.add_controller(keys);

        win.set_child(Some(&area));
        win.present();
        windows.borrow_mut().push(win);
    }
}
