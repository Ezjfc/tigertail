//! Local evdev helpers shared by the pen and touch readers.

use evdevil::event::{EventType, InputEvent};
use evdevil::Evdev;

use crate::pen::Result;

/// Open an input node, optionally grabbing it exclusively (`EVIOCGRAB`) so
/// xochitl stops receiving its events.
pub fn open(path: &str, grab: bool, what: &str) -> Result<Evdev> {
    let evdev = Evdev::open(path)?;
    log::info!(
        "Opened {} device {} ({})",
        what,
        path,
        evdev.name().unwrap_or_else(|_| "?".into())
    );
    if grab {
        evdev.grab()?;
        log::info!("Grabbed {} device exclusively (xochitl sees no {} input)", what, what);
    }
    Ok(evdev)
}

/// Grab a device and discard its events until interrupted. Used when a
/// device is not forwarded but should still be kept away from xochitl.
pub fn hold_grabbed(path: &str, what: &str) -> Result<()> {
    let evdev = open(path, true, what)?;
    log::info!("Holding {} device grabbed without forwarding", what);
    let mut buf = [InputEvent::new(EventType::from_raw(0), 0, 0); 64];
    loop {
        evdev.read_events(&mut buf)?;
    }
}

/// Print raw evdev events, one line per SYN_REPORT frame, until interrupted.
/// `tigertaild dump raw-pen` / `dump raw-touch` use this to see exactly what
/// the tablet's drivers emit (slot protocol quirks, ordering, timing).
pub fn dump_raw(path: &str, grab: bool, what: &str) -> Result<()> {
    let evdev = open(path, grab, what)?;
    let start = std::time::Instant::now();
    let mut buf = [InputEvent::new(EventType::from_raw(0), 0, 0); 64];
    let mut line: Vec<String> = Vec::new();
    loop {
        let n = evdev.read_events(&mut buf)?;
        for ev in &buf[..n] {
            let ty = ev.event_type().raw();
            let code = ev.raw_code();
            let value = ev.raw_value();
            if ty == 0 && code == 0 {
                println!("{:9.3} {}", start.elapsed().as_secs_f64(), line.join(" "));
                line.clear();
            } else {
                let name = match (ty, code) {
                    (1, c) => format!("KEY_{:#x}", c),
                    (3, 0x00) => "X".into(),
                    (3, 0x01) => "Y".into(),
                    (3, 0x18) => "PRESSURE".into(),
                    (3, 0x19) => "DISTANCE".into(),
                    (3, 0x1a) => "TILT_X".into(),
                    (3, 0x1b) => "TILT_Y".into(),
                    (3, 0x2f) => "SLOT".into(),
                    (3, 0x30) => "MT_MAJOR".into(),
                    (3, 0x35) => "MT_X".into(),
                    (3, 0x36) => "MT_Y".into(),
                    (3, 0x39) => "TRACK".into(),
                    (3, 0x3a) => "MT_PRESSURE".into(),
                    (3, c) => format!("ABS_{:#x}", c),
                    (t, c) => format!("T{}_{:#x}", t, c),
                };
                line.push(format!("{}={}", name, value));
            }
        }
    }
}
