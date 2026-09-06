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
