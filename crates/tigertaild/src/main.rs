mod config;
mod gadget;
mod hid;
mod pen;

use clap::Parser;

use config::{Cli, Command, Config};
use rm_pad::device::DeviceProfile;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let default_level = if cli.command.is_some() { "warn" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level))
        .init();

    let device = detect_device()?;
    let config = Config::load(&cli, device);

    if let Err(msg) = config.validate() {
        eprintln!("Error: {}", msg);
        eprintln!("\nRun with --help for usage information");
        std::process::exit(1);
    }

    match cli.command {
        Some(Command::Dump { device: which }) => match which.as_str() {
            "pen" => pen::run(&config, device, &mut pen::DumpSink),
            _ => {
                eprintln!("Unknown dump device: {}. Use 'pen'.", which);
                std::process::exit(1);
            }
        },
        None => {
            install_signal_handlers();
            let mut sink = gadget::HidSink::new();
            match pen::run(&config, device, &mut sink) {
                // A signal interrupting the evdev read is the clean shutdown
                // path; the sink's Drop restores the stock gadget.
                Err(e) if is_interrupted(&*e) => {
                    log::info!("Interrupted, shutting down");
                    Ok(())
                }
                other => other,
            }
        }
    }
}

fn is_interrupted(e: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::Interrupted)
}

/// SIGINT/SIGTERM/SIGHUP: no-op handlers without SA_RESTART, so blocking
/// reads return EINTR and the main loop unwinds through the teardown Drops.
/// SIGHUP matters because the controlling SSH session may ride the very USB
/// link the gadget re-enumerates.
fn install_signal_handlers() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = noop_handler as extern "C" fn(libc::c_int) as usize;
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
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
