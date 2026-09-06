//! Local pen event loop: reads the Wacom digitizer from /dev/input, applies
//! the rm_pad transform pipeline per frame, and hands complete frames to a
//! sink (debug dump now, USB HID reports later).
//!
//! This is the on-device port of rm-pad's host-side pen loop: same
//! frame-batching rules (defer X/Y/tilt to SYN_REPORT, tilt correction in raw
//! device space before orientation, tip synthesized from pressure), but events
//! come straight from evdev instead of an SSH stream, and output is
//! sink-agnostic instead of uinput.

use std::time::Instant;

use evdevil::event::{EventType, InputEvent, Key};

use rm_pad::device::DeviceProfile;
use rm_pad::display::SizeData;
use rm_pad::fit;
use rm_pad::palm::SharedPalmState;
use rm_pad::pen_map::{PenInputMap, PenInputPipeline};
use rm_pad::tilt;

use crate::config::Config;
use crate::evdev;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;
const ABS_DISTANCE: u16 = 0x19;
const ABS_TILT_X: u16 = 0x1a;
const ABS_TILT_Y: u16 = 0x1b;

/// Axis ranges of the transformed pen space, for sinks that must declare
/// logical ranges up front (HID report descriptors).
#[derive(Debug, Clone, Copy)]
pub struct PenGeometry {
    pub x_max: i32,
    pub y_max: i32,
    pub pressure_max: i32,
    /// Kept for parity with the device profile; no sink consumes it yet.
    #[allow(dead_code)]
    pub distance_max: i32,
    /// Tilt in device units; ±range maps to ±90°.
    pub tilt_range: i32,
}

/// One fully transformed pen state snapshot, emitted per SYN_REPORT.
#[derive(Debug, Clone, Copy, Default)]
pub struct PenFrame {
    pub x: i32,
    pub y: i32,
    pub pressure: i32,
    pub distance: i32,
    pub tilt_x: i32,
    pub tilt_y: i32,
    /// Pen coil in sensing range (BTN_TOOL_PEN or BTN_TOOL_RUBBER held).
    pub in_range: bool,
    /// Nib contact, synthesized from pressure like rm-pad does.
    pub tip: bool,
    /// Eraser tool active (BTN_TOOL_RUBBER).
    pub eraser: bool,
    /// Side button (BTN_STYLUS).
    pub barrel: bool,
}

pub trait PenSink {
    /// Called once before the event loop starts.
    fn begin(&mut self, geometry: &PenGeometry) -> Result<()>;
    /// Called with the complete pen state after every device frame.
    fn frame(&mut self, frame: &PenFrame) -> Result<()>;
}

/// The configured pen coordinate pipeline (orientation seed + fit stages).
fn pipeline(config: &Config, device: &DeviceProfile) -> PenInputPipeline {
    let (seed_x_max, seed_y_max) = config
        .orientation
        .pen_output_dimensions(device.pen_x_max, device.pen_y_max);

    let mut maps: Vec<Box<dyn PenInputMap>> = Vec::new();
    let size_data = config
        .aspect_ratio
        .map(SizeData::AspectRatio)
        .or(config.resolution.map(SizeData::Resolution));
    if let Some(fit_map) = fit::resolve(config.fit, size_data) {
        maps.push(Box::new(fit_map));
    }
    PenInputPipeline::new(seed_x_max, seed_y_max, maps)
}

/// Axis ranges the pen will report, as needed before the gadget attaches.
pub fn geometry(config: &Config, device: &DeviceProfile) -> PenGeometry {
    let pipeline = pipeline(config, device);
    PenGeometry {
        x_max: pipeline.axis_x_max,
        y_max: pipeline.axis_y_max,
        pressure_max: device.pen_pressure_max,
        distance_max: device.pen_distance_max,
        tilt_range: device.pen_tilt_range,
    }
}

pub fn run(
    config: &Config,
    device: &DeviceProfile,
    sink: &mut dyn PenSink,
    palm: Option<SharedPalmState>,
) -> Result<()> {
    let evdev = evdev::open(&config.pen_device, config.grab_pen, "pen")?;

    let pipeline = pipeline(config, device);
    log::info!("Pen input pipeline: {}", pipeline.describe());

    let correction = tilt::resolve(config.tilt_correction, config.tilt_correction_gain);
    if let Some(c) = &correction {
        log::info!("Pen tilt correction: {} (gain {})", c.mode, c.gain);
    }

    sink.begin(&geometry(config, device))?;

    // Raw (pre-transform) values pending for the current frame; tilt and
    // distance are not reported every frame, so remember the last values for
    // the tilt correction.
    let mut pending_x: Option<i32> = None;
    let mut pending_y: Option<i32> = None;
    let mut pending_tilt: Option<(i32, i32)> = None;
    let mut last_tilt_x = 0;
    let mut last_tilt_y = 0;

    let mut frame = PenFrame::default();
    let mut frame_count: u64 = 0;

    let mut buf = [InputEvent::new(EventType::from_raw(0), 0, 0); 64];

    loop {
        let n = evdev.read_events(&mut buf)?;

        for ev in &buf[..n] {
            let ty = ev.event_type().raw();
            let code = ev.raw_code();
            let value = ev.raw_value();

            match ty {
                EV_ABS => match code {
                    ABS_X => pending_x = Some(value),
                    ABS_Y => pending_y = Some(value),
                    ABS_TILT_X => {
                        last_tilt_x = value;
                        pending_tilt = Some((value, last_tilt_y));
                    }
                    ABS_TILT_Y => {
                        last_tilt_y = value;
                        pending_tilt = Some((last_tilt_x, value));
                    }
                    ABS_PRESSURE => frame.pressure = value,
                    ABS_DISTANCE => frame.distance = value,
                    _ => {}
                },
                EV_KEY => {
                    let pressed = value != 0;
                    if code == Key::BTN_TOOL_PEN.raw() {
                        frame.in_range = pressed;
                        if pressed {
                            frame.eraser = false;
                        }
                    } else if code == Key::BTN_TOOL_RUBBER.raw() {
                        frame.in_range = pressed;
                        frame.eraser = pressed;
                    } else if code == Key::BTN_STYLUS.raw() {
                        frame.barrel = pressed;
                    }
                }
                EV_SYN if code == SYN_REPORT => {
                    finish_frame(
                        &mut frame,
                        pending_x.take(),
                        pending_y.take(),
                        pending_tilt.take(),
                        (last_tilt_x, last_tilt_y),
                        &correction,
                        device,
                        &pipeline,
                        config,
                    );

                    if frame_count == 0 {
                        log::info!("Pen events flowing");
                    }
                    frame_count += 1;

                    update_palm_state(&palm, frame.tip);
                    sink.frame(&frame)?;
                }
                _ => {}
            }
        }
    }
}

/// Fold the pending raw values into `frame`, applying tilt correction (raw
/// device space), orientation, and the fit pipeline — the same fixed order as
/// rm-pad's host pen loop.
#[allow(clippy::too_many_arguments)]
fn finish_frame(
    frame: &mut PenFrame,
    pending_x: Option<i32>,
    pending_y: Option<i32>,
    pending_tilt: Option<(i32, i32)>,
    last_tilt: (i32, i32),
    correction: &Option<tilt::TiltCorrection>,
    device: &DeviceProfile,
    pipeline: &PenInputPipeline,
    config: &Config,
) {
    frame.tip = frame.pressure > 0;

    if let (Some(x), Some(y)) = (pending_x, pending_y) {
        let (x, y) = match correction {
            Some(c) => {
                let (dx, dy) = c.offset(
                    last_tilt.0,
                    last_tilt.1,
                    device.pen_tilt_range,
                    frame.distance,
                    device.pen_distance_max,
                );
                (
                    (x - dx).clamp(0, device.pen_x_max),
                    (y - dy).clamp(0, device.pen_y_max),
                )
            }
            None => (x, y),
        };
        let (ox, oy) =
            config
                .orientation
                .transform_pen(x, y, device.pen_x_max, device.pen_y_max);
        let (mx, my) = pipeline.map(ox, oy);
        frame.x = mx;
        frame.y = my;
    }

    if let Some((tx, ty)) = pending_tilt {
        let (otx, oty) = config.orientation.transform_tilt(tx, ty);
        frame.tilt_x = otx;
        frame.tilt_y = oty;
    }
}

/// Tell the touch loop whether the pen is down (rm-pad's palm rule).
fn update_palm_state(palm: &Option<SharedPalmState>, pen_down: bool) {
    let Some(palm_state) = palm else { return };
    let Ok(mut state) = palm_state.lock() else { return };
    state.pen_down = pen_down;
    if !pen_down {
        state.last_pen_up = Some(Instant::now());
    }
}

/// Sink for `tigertaild dump pen`: prints every frame.
pub struct DumpSink;

impl PenSink for DumpSink {
    fn begin(&mut self, geometry: &PenGeometry) -> Result<()> {
        println!("pen geometry: {:?}", geometry);
        Ok(())
    }

    fn frame(&mut self, f: &PenFrame) -> Result<()> {
        println!(
            "x={:5} y={:5} pressure={:4} distance={:3} tilt=({:5},{:5}) range={} tip={} eraser={} barrel={}",
            f.x,
            f.y,
            f.pressure,
            f.distance,
            f.tilt_x,
            f.tilt_y,
            f.in_range as u8,
            f.tip as u8,
            f.eraser as u8,
            f.barrel as u8,
        );
        Ok(())
    }
}
