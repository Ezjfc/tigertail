//! The settings window — also the app's main window.

use std::sync::Arc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;

use crate::config::{EraserAction, Orientation, Region, RegionSetting};
use crate::Shared;

pub fn build(app: &gtk::Application, shared: Arc<Shared>) -> gtk::ApplicationWindow {
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Chiral")
        .default_width(460)
        .default_height(620)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    // ---- status ----
    let status = gtk::Label::new(Some("Starting…"));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    root.append(&status);
    {
        let shared = shared.clone();
        let status = status.downgrade();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(status) = status.upgrade() else {
                return glib::ControlFlow::Break;
            };
            status.set_text(&shared.status());
            glib::ControlFlow::Continue
        });
    }

    // ---- region ----
    root.append(&heading("Mapping region"));
    let region_label = gtk::Label::new(None);
    region_label.set_xalign(0.0);
    region_label.set_text(&region_text(shared.config.read().unwrap().region));
    root.append(&region_label);

    let region_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let select_btn = gtk::Button::with_label("Select region…");
    let active_btn = gtk::Button::with_label("Active monitor");
    let desktop_btn = gtk::Button::with_label("Entire desktop");
    region_buttons.append(&select_btn);
    region_buttons.append(&active_btn);
    region_buttons.append(&desktop_btn);
    root.append(&region_buttons);

    let set_region = {
        let shared = shared.clone();
        let region_label = region_label.clone();
        move |setting: RegionSetting| {
            shared.config.write().unwrap().region = setting;
            shared.save_config();
            region_label.set_text(&region_text(setting));
        }
    };
    {
        let set_region = set_region.clone();
        let app = app.clone();
        select_btn.connect_clicked(move |_| {
            let set_region = set_region.clone();
            super::region::select(&app, move |result: Option<Region>| {
                if let Some(r) = result {
                    set_region(RegionSetting::Fixed(r));
                }
            });
        });
    }
    {
        let set_region = set_region.clone();
        active_btn.connect_clicked(move |_| set_region(RegionSetting::ActiveMonitor));
    }
    {
        let set_region = set_region.clone();
        desktop_btn.connect_clicked(move |_| set_region(RegionSetting::Desktop));
    }

    // ---- behaviour ----
    root.append(&heading("Behaviour"));
    let cfg = shared.config.read().unwrap().clone();

    let hover = gtk::Switch::new();
    hover.set_active(cfg.hover_moves_cursor);
    hover.set_valign(gtk::Align::Center);
    {
        let shared = shared.clone();
        hover.connect_active_notify(move |sw| {
            shared.config.write().unwrap().hover_moves_cursor = sw.is_active();
            shared.save_config();
        });
    }
    root.append(&row(
        "Hover moves cursor",
        "Cursor jumps to the pen as soon as the tip gets close",
        &hover,
    ));

    let pressure = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        0.0,
        crate::pen::PRESSURE_MAX,
        10.0,
    );
    pressure.set_hexpand(true);
    pressure.set_value(cfg.pressure_threshold as f64);
    {
        let shared = shared.clone();
        pressure.connect_value_changed(move |s| {
            shared.config.write().unwrap().pressure_threshold = s.value() as i32;
            shared.save_config();
        });
    }
    root.append(&row(
        "Pressure threshold",
        "Raw pressure needed to count as a click; 0 = use the tablet's own tip detection",
        &pressure,
    ));

    let eraser = gtk::DropDown::from_strings(&["No action", "Right click", "Middle click"]);
    eraser.set_selected(match cfg.eraser_action {
        EraserAction::None => 0,
        EraserAction::RightClick => 1,
        EraserAction::MiddleClick => 2,
    });
    {
        let shared = shared.clone();
        eraser.connect_selected_notify(move |dd| {
            shared.config.write().unwrap().eraser_action = match dd.selected() {
                0 => EraserAction::None,
                2 => EraserAction::MiddleClick,
                _ => EraserAction::RightClick,
            };
            shared.save_config();
        });
    }
    root.append(&row(
        "Eraser tool",
        "What a Marker Plus eraser press does",
        &eraser,
    ));

    let smoothing = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 0.95, 0.05);
    smoothing.set_hexpand(true);
    smoothing.set_value(cfg.smoothing);
    {
        let shared = shared.clone();
        smoothing.connect_value_changed(move |s| {
            shared.config.write().unwrap().smoothing = s.value();
            shared.save_config();
        });
    }
    root.append(&row(
        "Smoothing",
        "Position smoothing; 0 = raw, higher = steadier but laggier",
        &smoothing,
    ));

    // ---- mapping ----
    root.append(&heading("Mapping"));

    let orientation = gtk::DropDown::from_strings(&[
        "Portrait",
        "Portrait flipped",
        "Landscape left",
        "Landscape right",
    ]);
    orientation.set_selected(match cfg.orientation {
        Orientation::Portrait => 0,
        Orientation::PortraitFlipped => 1,
        Orientation::LandscapeLeft => 2,
        Orientation::LandscapeRight => 3,
    });
    {
        let shared = shared.clone();
        orientation.connect_selected_notify(move |dd| {
            shared.config.write().unwrap().orientation = match dd.selected() {
                0 => Orientation::Portrait,
                1 => Orientation::PortraitFlipped,
                3 => Orientation::LandscapeRight,
                _ => Orientation::LandscapeLeft,
            };
            shared.save_config();
        });
    }
    root.append(&row(
        "Orientation",
        "How the tablet is physically placed; cycle these (and the inverts) if motion comes out rotated or mirrored",
        &orientation,
    ));

    let inverts = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let invert_x = gtk::CheckButton::with_label("Invert X");
    invert_x.set_active(cfg.invert_x);
    let invert_y = gtk::CheckButton::with_label("Invert Y");
    invert_y.set_active(cfg.invert_y);
    {
        let shared = shared.clone();
        invert_x.connect_toggled(move |c| {
            shared.config.write().unwrap().invert_x = c.is_active();
            shared.save_config();
        });
    }
    {
        let shared = shared.clone();
        invert_y.connect_toggled(move |c| {
            shared.config.write().unwrap().invert_y = c.is_active();
            shared.save_config();
        });
    }
    inverts.append(&invert_x);
    inverts.append(&invert_y);
    root.append(&inverts);

    let lock_aspect = gtk::Switch::new();
    lock_aspect.set_active(cfg.lock_aspect);
    lock_aspect.set_valign(gtk::Align::Center);
    {
        let shared = shared.clone();
        lock_aspect.connect_active_notify(move |sw| {
            shared.config.write().unwrap().lock_aspect = sw.is_active();
            shared.save_config();
        });
    }
    root.append(&row(
        "Lock aspect ratio",
        "Letterbox the tablet's 4:3 area inside the region instead of stretching",
        &lock_aspect,
    ));

    // ---- connection ----
    root.append(&heading("Connection"));

    let host = gtk::Entry::new();
    host.set_text(&cfg.host);
    {
        let shared = shared.clone();
        host.connect_changed(move |e| {
            shared.config.write().unwrap().host = e.text().to_string();
            shared.save_config();
        });
    }
    root.append(&row("Host", "10.11.99.1 over USB", &host));

    let user = gtk::Entry::new();
    user.set_text(&cfg.user);
    {
        let shared = shared.clone();
        user.connect_changed(move |e| {
            shared.config.write().unwrap().user = e.text().to_string();
            shared.save_config();
        });
    }
    root.append(&row("User", "", &user));

    let device = gtk::Entry::new();
    device.set_placeholder_text(Some("auto-detect"));
    device.set_text(cfg.device.as_deref().unwrap_or(""));
    {
        let shared = shared.clone();
        device.connect_changed(move |e| {
            let text = e.text().trim().to_string();
            shared.config.write().unwrap().device = if text.is_empty() { None } else { Some(text) };
            shared.save_config();
        });
    }
    root.append(&row(
        "Device",
        "Evdev path on the tablet, e.g. /dev/input/event1; blank = auto-detect",
        &device,
    ));

    let reconnect = gtk::Button::with_label("Reconnect");
    reconnect.set_halign(gtk::Align::Start);
    {
        let shared = shared.clone();
        reconnect.connect_clicked(move |_| shared.request_reconnect());
    }
    root.append(&reconnect);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&root));
    win.set_child(Some(&scroll));
    win
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>{text}</b>"));
    l.set_xalign(0.0);
    l.set_margin_top(12);
    l
}

fn row(title: &str, subtitle: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_xalign(0.0);
    text.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.add_css_class("dim-label");
        s.add_css_class("caption");
        text.append(&s);
    }
    outer.append(&text);
    let c = control.clone().upcast::<gtk::Widget>();
    if c.is::<gtk::Scale>() || c.is::<gtk::Entry>() {
        c.set_size_request(180, -1);
    }
    outer.append(&c);
    outer
}

fn region_text(setting: RegionSetting) -> String {
    match setting {
        RegionSetting::ActiveMonitor => "Whole active monitor (default)".into(),
        RegionSetting::Desktop => "Entire desktop".into(),
        RegionSetting::Fixed(r) => format!(
            "{} × {} at ({}, {})",
            r.w.round() as i32,
            r.h.round() as i32,
            r.x.round() as i32,
            r.y.round() as i32
        ),
    }
}
