//! Reads the reMarkable 2's Wacom digitizer over SSH and turns the raw evdev
//! stream into [`PenSnapshot`]s.
//!
//! The tablet needs no extra software: `cat /dev/input/eventN` over SSH yields
//! the kernel input events directly. The rM2 is 32-bit ARM, so each record is
//! 16 bytes little-endian: u32 tv_sec, u32 tv_usec, u16 type, u16 code,
//! i32 value.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Shared;

/// rM2 Wacom digitizer axis ranges. X runs along the long edge of the device.
pub const SENSOR_X_MAX: f64 = 20966.0;
pub const SENSOR_Y_MAX: f64 = 15725.0;
pub const PRESSURE_MAX: f64 = 4095.0;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOOL_RUBBER: u16 = 0x141;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;

#[derive(Clone, Copy, Debug, Default)]
pub struct PenSnapshot {
    pub x: i32,
    pub y: i32,
    pub pressure: i32,
    /// Pen (or eraser) is close enough to the surface to be tracked.
    pub in_range: bool,
    /// The eraser end of the marker is the active tool.
    pub eraser: bool,
    /// The tablet's own tip-contact detection (BTN_TOUCH).
    pub touch: bool,
}

pub fn run(shared: Arc<Shared>, tx: Sender<PenSnapshot>) {
    loop {
        let (host, user, device_override) = {
            let c = shared.config.read().unwrap();
            (c.host.clone(), c.user.clone(), c.device.clone())
        };
        let dest = format!("{user}@{host}");

        shared.set_status(format!("Connecting to {dest}…"));
        let device = match device_override.filter(|d| !d.trim().is_empty()) {
            Some(d) => Ok(d.trim().to_string()),
            None => autodetect(&dest),
        };
        let device = match device {
            Ok(d) => d,
            Err(e) => {
                shared.set_status(format!("{e} — retrying in 3 s"));
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };

        let result = stream(&shared, &dest, &device, &tx);
        // Make sure the desktop is not left with a stuck button.
        if tx.send(PenSnapshot::default()).is_err() {
            return; // injector is gone, nothing left to do
        }
        match result {
            Ok(()) => shared.set_status("Disconnected — reconnecting in 3 s".into()),
            Err(e) => shared.set_status(format!("Connection lost: {e} — reconnecting in 3 s")),
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn ssh_base(dest: &str) -> Command {
    let mut c = Command::new("ssh");
    c.args([
        "-o",
        "ConnectTimeout=5",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        // Fail instead of prompting for a password; keys are required anyway
        // since there is no TTY here.
        "-o",
        "BatchMode=yes",
        dest,
    ]);
    c.stdin(Stdio::null());
    c
}

fn autodetect(dest: &str) -> anyhow::Result<String> {
    let script = r#"for d in /sys/class/input/event*; do case "$(cat $d/device/name)" in *[Ww]acom*) echo "/dev/input/$(basename $d)";; esac; done"#;
    let out = ssh_base(dest)
        .arg(script)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run ssh: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let hint = if err.contains("Permission denied") {
            " (set up key auth: ssh-copy-id, password is on the tablet under Settings → Help)"
        } else {
            ""
        };
        anyhow::bail!("ssh to {dest} failed: {}{hint}", err.trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    match stdout.lines().next() {
        Some(dev) if dev.starts_with("/dev/input/") => Ok(dev.to_string()),
        _ => anyhow::bail!("no Wacom digitizer found on {dest}"),
    }
}

fn stream(
    shared: &Shared,
    dest: &str,
    device: &str,
    tx: &Sender<PenSnapshot>,
) -> anyhow::Result<()> {
    let mut child = ssh_base(dest)
        .arg(format!("cat {device}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to run ssh: {e}"))?;
    *shared.ssh_pid.lock().unwrap() = Some(child.id());

    // Keep the last stderr line around for a useful error message on EOF.
    let last_err: Arc<Mutex<String>> = Arc::default();
    if let Some(stderr) = child.stderr.take() {
        let last_err = last_err.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                *last_err.lock().unwrap() = line;
            }
        });
    }

    shared.set_status(format!("Connected — streaming {device}"));

    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut state = PenState::default();
    let mut buf = [0u8; 16];
    let result = loop {
        if let Err(e) = stdout.read_exact(&mut buf) {
            break Err(e);
        }
        if let Some(snap) = state.feed(&buf) {
            if tx.send(snap).is_err() {
                break Ok(()); // injector is gone
            }
        }
    };

    *shared.ssh_pid.lock().unwrap() = None;
    let _ = child.kill();
    let _ = child.wait();

    match result {
        Ok(()) => Ok(()),
        Err(_) => {
            let err = last_err.lock().unwrap().clone();
            if err.is_empty() {
                anyhow::bail!("stream ended")
            } else {
                anyhow::bail!("{err}")
            }
        }
    }
}

#[derive(Default)]
struct PenState {
    x: i32,
    y: i32,
    pressure: i32,
    pen_in: bool,
    rubber_in: bool,
    touch: bool,
}

impl PenState {
    /// Feed one 16-byte input_event record; returns a snapshot on EV_SYN.
    fn feed(&mut self, buf: &[u8; 16]) -> Option<PenSnapshot> {
        let typ = u16::from_le_bytes([buf[8], buf[9]]);
        let code = u16::from_le_bytes([buf[10], buf[11]]);
        let value = i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        match (typ, code) {
            (EV_ABS, ABS_X) => self.x = value,
            (EV_ABS, ABS_Y) => self.y = value,
            (EV_ABS, ABS_PRESSURE) => self.pressure = value,
            (EV_KEY, BTN_TOOL_PEN) => self.pen_in = value != 0,
            (EV_KEY, BTN_TOOL_RUBBER) => self.rubber_in = value != 0,
            (EV_KEY, BTN_TOUCH) => self.touch = value != 0,
            (EV_SYN, _) => {
                return Some(PenSnapshot {
                    x: self.x,
                    y: self.y,
                    pressure: self.pressure,
                    in_range: self.pen_in || self.rubber_in,
                    eraser: self.rubber_in,
                    touch: self.touch,
                })
            }
            _ => {}
        }
        None
    }
}
