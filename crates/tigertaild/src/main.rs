mod config;
mod evdev;
mod gadget;
mod hid;
mod pen;
mod touch;

use std::os::unix::thread::JoinHandleExt;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use clap::Parser;

use config::{Cli, Command, Config};
use gadget::{Gadget, HidInterface, NoFeatures, ReportSink, UsbStrings};
use pen::{PenFrame, PenGeometry, PenSink};
use rm_pad::device::DeviceProfile;
use rm_pad::palm::{PalmState, SharedPalmState};
use touch::{TouchFrame, TouchGeometry, TouchSink};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_level = if cli.command.is_some() { "warn" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level))
        .init();

    let device = detect_device()?;
    let config = Config::load(&cli, device).unwrap_or_else(|msg| {
        eprintln!("Error: {}", msg);
        std::process::exit(1);
    });

    if let Some(Command::Dump { device: which }) = cli.command {
        // Dumps read one device in the foreground; a signal EINTRs the read.
        install_noop_handler(&[libc::SIGINT, libc::SIGTERM, libc::SIGHUP]);
        let result = match which.as_str() {
            "pen" => pen::run(&config, device, &mut pen::DumpSink, None),
            "touch" => touch::run(&config, device, &mut touch::DumpSink::new(), None),
            _ => {
                eprintln!("Unknown dump device: {}. Use 'pen' or 'touch'.", which);
                std::process::exit(1);
            }
        };
        return match result {
            Err(e) if is_interrupted(&*e) => Ok(()),
            other => other,
        };
    }

    if let Err(msg) = config.validate() {
        eprintln!("Error: {}", msg);
        eprintln!("\nRun with --help for usage information");
        std::process::exit(1);
    }

    log_startup_info(&config);
    run_gadget(config, device)
}

fn log_startup_info(config: &Config) {
    let on_off = |b: bool| if b { "on" } else { "off" };
    log::info!(
        "Starting tigertaild: pen={} (grab {}), touch={} (grab {}), palm_rejection={}, orientation={}, fit={}, aspect_ratio={}, resolution={}, tilt_correction={}",
        on_off(config.pen),
        on_off(config.grab_pen),
        on_off(config.touch),
        on_off(config.grab_touch),
        if config.palm_rejection { format!("on ({}ms)", config.palm_grace_ms) } else { "off".into() },
        config.orientation,
        config.fit,
        config.aspect_ratio.map_or("none".to_string(), |a| a.to_string()),
        config.resolution.map_or("none".to_string(), |r| r.to_string()),
        config.tilt_correction,
    );
}

/// Attach the gadget and run the pen/touch workers until a signal arrives.
///
/// Shutdown signals are blocked in every thread and collected by `sigwait`
/// here, so the teardown order is deterministic: interrupt the workers'
/// blocking evdev reads with [`gadget::WAKE_SIGNAL`], join them, then drop
/// the gadget (which stops ep0 and restores the stock configfs tree).
fn run_gadget(config: Config, device: &'static DeviceProfile) -> Result<()> {
    block_shutdown_signals();
    install_noop_handler(&[gadget::WAKE_SIGNAL]);

    let config = Arc::new(config);
    let mut interfaces = Vec::new();
    let mut pen_index = None;
    let mut touch_index = None;

    if config.pen {
        let geo = pen::geometry(&config, device);
        pen_index = Some(interfaces.len());
        interfaces.push(HidInterface {
            name: "pen",
            report_descriptor: hid::pen::report_descriptor(&geo),
            in_report_len: hid::pen::REPORT_LEN,
            features: Box::new(NoFeatures),
        });
    }
    if config.touch {
        let geo = touch::geometry(&config, device);
        touch_index = Some(interfaces.len());
        interfaces.push(HidInterface {
            name: "touchpad",
            report_descriptor: hid::touchpad::report_descriptor(&geo),
            in_report_len: hid::touchpad::INPUT_REPORT_LEN,
            features: Box::new(hid::touchpad::Features::new()),
        });
    }

    let strings = UsbStrings {
        product: config.usb_product.clone(),
        manufacturer: config.usb_manufacturer.clone(),
    };
    let mut gadget = Gadget::attach(interfaces, &strings)?;

    let palm: Option<SharedPalmState> = if config.pen && config.touch && config.palm_rejection {
        Some(Arc::new(Mutex::new(PalmState::new())))
    } else {
        None
    };

    let mut workers: Vec<JoinHandle<()>> = Vec::new();

    if let Some(i) = pen_index {
        let sink = gadget.sink(i);
        let (config, palm) = (config.clone(), palm.clone());
        workers.push(spawn_worker("pen", move || {
            let mut sink = PenHidSink { sink, geo: pen::geometry(&config, device) };
            pen::run(&config, device, &mut sink, palm)
        }));
    } else if config.grab_pen {
        let config = config.clone();
        workers.push(spawn_worker("pen-grab", move || {
            evdev::hold_grabbed(&config.pen_device, "pen")
        }));
    }

    if let Some(i) = touch_index {
        let sink = gadget.sink(i);
        let (config, palm) = (config.clone(), palm.clone());
        workers.push(spawn_worker("touch", move || {
            let mut sink = TouchHidSink { sink, start: Instant::now() };
            touch::run(&config, device, &mut sink, palm)
        }));
    } else if config.grab_touch {
        let config = config.clone();
        workers.push(spawn_worker("touch-grab", move || {
            evdev::hold_grabbed(&config.touch_device, "touch")
        }));
    }

    let sig = wait_shutdown_signal();
    log::info!("Received signal {}, shutting down", sig);

    for w in &workers {
        unsafe {
            libc::pthread_kill(w.as_pthread_t() as usize as libc::pthread_t, gadget::WAKE_SIGNAL);
        }
    }
    for w in workers {
        let _ = w.join();
    }
    drop(gadget);
    Ok(())
}

/// Run a worker; any failure other than our own wake-up signal asks the
/// main thread to shut everything down.
fn spawn_worker<F>(name: &'static str, f: F) -> JoinHandle<()>
where
    F: FnOnce() -> Result<()> + Send + 'static,
{
    thread::Builder::new()
        .name(name.into())
        .spawn(move || match f() {
            Ok(()) => {}
            Err(e) if is_interrupted(&*e) => log::debug!("[{}] interrupted", name),
            Err(e) => {
                log::error!("[{}] {}", name, e);
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGTERM);
                }
            }
        })
        .expect("spawn worker")
}

struct PenHidSink {
    sink: ReportSink,
    geo: PenGeometry,
}

impl PenSink for PenHidSink {
    fn begin(&mut self, _: &PenGeometry) -> Result<()> {
        Ok(())
    }
    fn frame(&mut self, frame: &PenFrame) -> Result<()> {
        self.sink.send(&hid::pen::pack_report(frame, &self.geo))
    }
}

struct TouchHidSink {
    sink: ReportSink,
    start: Instant,
}

impl TouchSink for TouchHidSink {
    fn begin(&mut self, _: &TouchGeometry) -> Result<()> {
        Ok(())
    }
    fn frame(&mut self, frame: &TouchFrame) -> Result<()> {
        // PTP scan time: 100 µs units, free-running, wraps at 16 bits.
        let scan = (self.start.elapsed().as_micros() / 100) as u16;
        self.sink.send(&hid::touchpad::pack_report(frame, scan))
    }
}

fn is_interrupted(e: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::Interrupted)
}

const SHUTDOWN_SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

fn shutdown_sigset() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in SHUTDOWN_SIGNALS {
            libc::sigaddset(&mut set, sig);
        }
        set
    }
}

/// Block the shutdown signals in this thread; threads spawned afterwards
/// inherit the mask, so only `wait_shutdown_signal` ever consumes them.
/// SIGHUP matters because the controlling SSH session may ride the very USB
/// link the gadget re-enumerates.
fn block_shutdown_signals() {
    let set = shutdown_sigset();
    unsafe {
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

fn wait_shutdown_signal() -> libc::c_int {
    let set = shutdown_sigset();
    let mut sig: libc::c_int = 0;
    unsafe {
        libc::sigwait(&set, &mut sig);
    }
    sig
}

/// No-op handlers without SA_RESTART, so blocking reads return EINTR.
fn install_noop_handler(signals: &[libc::c_int]) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = noop_handler as extern "C" fn(libc::c_int) as usize;
        for &sig in signals {
            libc::sigaction(sig, &action, std::ptr::null_mut());
        }
    }
}

extern "C" fn noop_handler(_sig: libc::c_int) {}

/// Identify the tablet from the local device tree.
fn detect_device() -> Result<&'static DeviceProfile> {
    let model = std::fs::read_to_string("/proc/device-tree/model")?;
    let device = DeviceProfile::from_model(&model)?;
    log::info!("Device profile: {}", device.name);
    Ok(device)
}
