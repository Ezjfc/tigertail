mod config;
mod inject;
mod pen;
mod ui;

use std::sync::{mpsc, Arc, Mutex, RwLock};

use gtk4 as gtk;
use gtk4::prelude::*;

use config::Config;

pub struct Shared {
    pub config: RwLock<Config>,
    status: Mutex<String>,
    /// PID of the running ssh stream, so the UI can force a reconnect.
    pub ssh_pid: Mutex<Option<u32>>,
}

impl Shared {
    fn new(config: Config) -> Self {
        Self {
            config: RwLock::new(config),
            status: Mutex::new("Starting…".into()),
            ssh_pid: Mutex::new(None),
        }
    }

    pub fn set_status(&self, s: String) {
        eprintln!("chiral: {s}");
        *self.status.lock().unwrap() = s;
    }

    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    pub fn save_config(&self) {
        let cfg = self.config.read().unwrap().clone();
        if let Err(e) = cfg.save() {
            eprintln!("chiral: failed to save config: {e}");
        }
    }

    /// Kill the current ssh stream; the reader loop reconnects with the
    /// current settings.
    pub fn request_reconnect(&self) {
        if let Some(pid) = *self.ssh_pid.lock().unwrap() {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .status();
        }
    }
}

fn main() -> gtk::glib::ExitCode {
    let shared = Arc::new(Shared::new(Config::load()));

    let (tx, rx) = mpsc::channel();
    {
        let shared = shared.clone();
        std::thread::spawn(move || pen::run(shared, tx));
    }
    {
        let shared = shared.clone();
        std::thread::spawn(move || inject::run(shared, rx));
    }

    let app = gtk::Application::builder()
        .application_id("dev.chiral.Chiral")
        .build();

    app.connect_startup(|app| {
        // Background daemon: stay alive with no windows. The guard is held for
        // the lifetime of the process.
        std::mem::forget(app.hold());

        if let Some(display) = gtk::gdk::Display::default() {
            let css = gtk::CssProvider::new();
            css.load_from_string("window.chiral-overlay { background: transparent; }");
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    {
        let shared = shared.clone();
        // Also fired when a second `chiral` launch activates this instance:
        // re-present the window instead of starting anew.
        app.connect_activate(move |app| {
            match app
                .windows()
                .into_iter()
                .find(|w| w.title().is_some_and(|t| t == "Chiral"))
            {
                Some(w) => w.present(),
                None => ui::settings::build(app, shared.clone()).present(),
            }
        });
    }

    app.run()
}
