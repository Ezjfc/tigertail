//! Local touch event loop: reads the capacitive panel from /dev/input,
//! tracks multitouch slots, applies orientation, and hands complete frames
//! to a sink (debug dump, or the PTP HID interface).
//!
//! On-device port of rm-pad's host-side touch loop: same slot bookkeeping
//! (`ABS_MT_SLOT`/`TRACKING_ID`/`POSITION_*`, positional fallback when the
//! driver omits slot events) and the same palm-rejection rule.

use std::time::Instant;

use evdevil::event::{EventType, InputEvent};

use rm_pad::device::DeviceProfile;
use rm_pad::orientation::Orientation;
use rm_pad::palm::SharedPalmState;

use crate::config::Config;
use crate::evdev;
use crate::pen::Result;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;

/// Device-side slot count tracked (rM2's driver uses low slots first).
const MT_SLOTS: usize = 16;
/// Contacts reported to the host per frame (PTP-typical; gestures need ≤4).
pub const MAX_CONTACTS: usize = 5;

/// Axis ranges of the oriented touch space, for the HID descriptor.
#[derive(Debug, Clone, Copy)]
pub struct TouchGeometry {
    pub x_max: i32,
    pub y_max: i32,
    /// Units per millimetre.
    pub resolution: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Contact {
    /// Stable per-touch identifier (the evdev slot index).
    pub id: u8,
    pub tip: bool,
    pub x: i32,
    pub y: i32,
}

/// One oriented touch snapshot, emitted per device frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TouchFrame {
    pub contacts: [Contact; MAX_CONTACTS],
    /// Number of contacts with `tip` set.
    pub count: u8,
}

pub trait TouchSink {
    fn begin(&mut self, geometry: &TouchGeometry) -> Result<()>;
    fn frame(&mut self, frame: &TouchFrame) -> Result<()>;
}

pub fn geometry(config: &Config, device: &DeviceProfile) -> TouchGeometry {
    let (x_max, y_max) = config
        .orientation
        .touch_output_dimensions(device.touch_x_max, device.touch_y_max);
    TouchGeometry { x_max, y_max, resolution: device.touch_resolution }
}

struct SlotState {
    x: [Option<i32>; MT_SLOTS],
    y: [Option<i32>; MT_SLOTS],
    active: [bool; MT_SLOTS],
}

impl SlotState {
    fn new() -> Self {
        Self { x: [None; MT_SLOTS], y: [None; MT_SLOTS], active: [false; MT_SLOTS] }
    }

    fn active_count(&self) -> usize {
        self.active.iter().filter(|&&a| a).count()
    }
}

struct FrameState {
    current_slot: usize,
    /// (x, y) pairs seen this frame, for drivers that report positions
    /// without slot events (rm-pad's positional fallback).
    pending_positions: Vec<(i32, i32)>,
}

pub fn run(
    config: &Config,
    device: &DeviceProfile,
    sink: &mut dyn TouchSink,
    palm: Option<SharedPalmState>,
) -> Result<()> {
    let evdev = evdev::open(&config.touch_device, config.grab_touch, "touch")?;
    let orientation = config.orientation;
    let geo = geometry(config, device);
    sink.begin(&geo)?;

    let mut slots = SlotState::new();
    let mut frame_state = FrameState { current_slot: 0, pending_positions: Vec::with_capacity(MT_SLOTS) };
    let mut last_sent = TouchFrame::default();
    let mut frame_count: u64 = 0;
    let mut buf = [InputEvent::new(EventType::from_raw(0), 0, 0); 64];

    loop {
        let n = evdev.read_events(&mut buf)?;
        for ev in &buf[..n] {
            let ty = ev.event_type().raw();
            let code = ev.raw_code();
            let value = ev.raw_value();
            match ty {
                EV_KEY => {}
                EV_ABS => process_abs_event(&mut slots, &mut frame_state, code, value),
                EV_SYN if code == SYN_REPORT => {
                    resolve_pending_positions(&mut slots, &frame_state);
                    frame_state.pending_positions.clear();

                    let frame = if should_suppress_palm(&palm, config.palm_grace_ms) {
                        TouchFrame::default()
                    } else {
                        build_frame(&slots, device, orientation, &geo)
                    };

                    if frame_count == 0 {
                        log::info!("Touch events flowing");
                    }
                    frame_count += 1;

                    // Hosts only need a report when something changed, plus
                    // one final all-released report.
                    if frame != last_sent {
                        sink.frame(&frame)?;
                        last_sent = frame;
                    }
                }
                _ => {}
            }
        }
    }
}

fn process_abs_event(slots: &mut SlotState, frame: &mut FrameState, code: u16, value: i32) {
    match code {
        ABS_MT_SLOT => {
            frame.current_slot = (value.max(0) as usize).min(MT_SLOTS - 1);
        }
        ABS_MT_TRACKING_ID => {
            let slot = frame.current_slot;
            if value >= 0 {
                slots.active[slot] = true;
            } else {
                slots.active[slot] = false;
                slots.x[slot] = None;
                slots.y[slot] = None;
            }
        }
        ABS_MT_POSITION_X => {
            let slot = frame.current_slot;
            slots.x[slot] = Some(value);
            slots.active[slot] = true;
        }
        ABS_MT_POSITION_Y => {
            let slot = frame.current_slot;
            slots.y[slot] = Some(value);
            slots.active[slot] = true;
            if let Some(x) = slots.x[slot] {
                frame.pending_positions.push((x, value));
            }
        }
        _ => {}
    }
}

/// When a frame carried exactly one position per active slot, assign them
/// in slot order (covers drivers that omit `ABS_MT_SLOT` between contacts).
fn resolve_pending_positions(slots: &mut SlotState, frame: &FrameState) {
    let active: Vec<usize> = (0..MT_SLOTS).filter(|&s| slots.active[s]).collect();
    if active.is_empty() || frame.pending_positions.len() != active.len() {
        return;
    }
    for (i, &slot) in active.iter().enumerate() {
        let (x, y) = frame.pending_positions[i];
        slots.x[slot] = Some(x);
        slots.y[slot] = Some(y);
    }
}

fn build_frame(
    slots: &SlotState,
    device: &DeviceProfile,
    orientation: Orientation,
    geo: &TouchGeometry,
) -> TouchFrame {
    let mut frame = TouchFrame::default();
    for slot in 0..MAX_CONTACTS {
        let contact = &mut frame.contacts[slot];
        contact.id = slot as u8;
        let (Some(x), Some(y)) = (slots.x[slot], slots.y[slot]) else { continue };
        if !slots.active[slot] {
            continue;
        }
        let (ox, oy) = orientation.transform_touch(
            x.clamp(0, device.touch_x_max),
            y.clamp(0, device.touch_y_max),
            device.touch_x_max,
            device.touch_y_max,
        );
        contact.tip = true;
        contact.x = ox.clamp(0, geo.x_max);
        contact.y = oy.clamp(0, geo.y_max);
        frame.count += 1;
    }
    if slots.active_count() > MAX_CONTACTS {
        log::debug!("More than {} contacts; extra slots ignored", MAX_CONTACTS);
    }
    frame
}

fn should_suppress_palm(palm: &Option<SharedPalmState>, grace_ms: u64) -> bool {
    let Some(palm_state) = palm else { return false };
    let Ok(state) = palm_state.lock() else { return false };
    if state.pen_down {
        return true;
    }
    state
        .last_pen_up
        .map(|t| t.elapsed().as_millis() < grace_ms as u128)
        .unwrap_or(false)
}

/// Sink for `tigertaild dump touch`.
pub struct DumpSink {
    start: Instant,
}

impl DumpSink {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl TouchSink for DumpSink {
    fn begin(&mut self, geometry: &TouchGeometry) -> Result<()> {
        println!("touch geometry: {:?}", geometry);
        Ok(())
    }

    fn frame(&mut self, f: &TouchFrame) -> Result<()> {
        let contacts: Vec<String> = f
            .contacts
            .iter()
            .filter(|c| c.tip)
            .map(|c| format!("#{}({},{})", c.id, c.x, c.y))
            .collect();
        println!(
            "t={:8.3} count={} {}",
            self.start.elapsed().as_secs_f64(),
            f.count,
            contacts.join(" ")
        );
        Ok(())
    }
}
