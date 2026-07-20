//! Maps pen snapshots onto a screen region and injects them as absolute
//! pointer events through the wlr-virtual-pointer Wayland protocol.
//!
//! This runs on its own Wayland connection (separate from GTK's). Output
//! geometry comes from xdg-output in logical coordinates; absolute motion is
//! expressed over the bounding box of the whole output layout, which is how
//! Hyprland (and wlroots compositors generally) interpret it.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{delegate_noop, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use crate::config::{self, Config, EraserAction, Orientation, Region, RegionSetting};
use crate::pen::{PenSnapshot, SENSOR_X_MAX, SENSOR_Y_MAX};
use crate::Shared;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

pub fn run(shared: Arc<Shared>, rx: Receiver<PenSnapshot>) {
    if let Err(e) = run_inner(&shared, rx) {
        shared.set_status(format!("Wayland error: {e}"));
    }
}

fn run_inner(shared: &Shared, rx: Receiver<PenSnapshot>) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env().context("no Wayland display")?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());

    let mut app = App::default();
    queue.roundtrip(&mut app)?;
    app.ensure_xdg_outputs(&qh);
    queue.roundtrip(&mut app)?;

    let manager = app
        .manager
        .clone()
        .context("compositor does not support wlr-virtual-pointer-unstable-v1")?;
    let pointer = manager.create_virtual_pointer(app.seat.as_ref(), &qh, ());
    conn.flush()?;

    let mut inj = Injector {
        pointer,
        start: Instant::now(),
        held: [false; 3],
        ema: None,
        was_in_range: false,
        region: None,
    };

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(snap) => {
                let cfg = shared.config.read().unwrap().clone();
                inj.handle(&snap, &cfg, &app);
                conn.flush()?;
            }
            Err(RecvTimeoutError::Timeout) => {
                // Pick up output hotplug / geometry changes while idle.
                queue.roundtrip(&mut app)?;
                app.ensure_xdg_outputs(&qh);
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

#[derive(Default)]
struct OutputInfo {
    wl: Option<WlOutput>,
    xdg: Option<ZxdgOutputV1>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    name: String,
}

#[derive(Default)]
struct App {
    seat: Option<WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
    xdg_manager: Option<ZxdgOutputManagerV1>,
    outputs: HashMap<u32, OutputInfo>,
}

impl App {
    fn ensure_xdg_outputs(&mut self, qh: &QueueHandle<Self>) {
        let Some(mgr) = &self.xdg_manager else { return };
        for (&id, info) in self.outputs.iter_mut() {
            if info.xdg.is_none() {
                if let Some(wl) = &info.wl {
                    info.xdg = Some(mgr.get_xdg_output(wl, qh, id));
                }
            }
        }
    }

    /// Bounding box of all outputs in global logical coordinates.
    fn layout_bbox(&self) -> Region {
        let mut it = self.outputs.values().filter(|o| o.w > 0 && o.h > 0);
        let Some(first) = it.next() else {
            return Region {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
            };
        };
        let (mut x0, mut y0) = (first.x, first.y);
        let (mut x1, mut y1) = (first.x + first.w, first.y + first.h);
        for o in it {
            x0 = x0.min(o.x);
            y0 = y0.min(o.y);
            x1 = x1.max(o.x + o.w);
            y1 = y1.max(o.y + o.h);
        }
        Region {
            x: x0 as f64,
            y: y0 as f64,
            w: (x1 - x0) as f64,
            h: (y1 - y0) as f64,
        }
    }
}

struct Injector {
    pointer: ZwlrVirtualPointerV1,
    start: Instant,
    /// left, right, middle
    held: [bool; 3],
    ema: Option<(f64, f64)>,
    was_in_range: bool,
    region: Option<Region>,
}

impl Injector {
    fn now(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn handle(&mut self, s: &PenSnapshot, cfg: &Config, app: &App) {
        let bbox = app.layout_bbox();

        // Re-resolve the region every time the pen comes into range, so
        // "active monitor" follows focus between strokes.
        if s.in_range && !self.was_in_range {
            self.region = Some(resolve_region(cfg, &bbox));
            self.ema = None;
        }
        self.was_in_range = s.in_range;

        if !s.in_range {
            self.release_all();
            return;
        }

        let contact = if cfg.pressure_threshold > 0 {
            s.pressure >= cfg.pressure_threshold
        } else {
            s.touch
        };

        let a = (s.x as f64 / SENSOR_X_MAX).clamp(0.0, 1.0);
        let b = (s.y as f64 / SENSOR_Y_MAX).clamp(0.0, 1.0);
        let (mut u, mut v) = match cfg.orientation {
            Orientation::Portrait => (1.0 - b, 1.0 - a),
            Orientation::PortraitFlipped => (b, a),
            Orientation::LandscapeLeft => (1.0 - a, b),
            Orientation::LandscapeRight => (a, 1.0 - b),
        };
        if cfg.invert_x {
            u = 1.0 - u;
        }
        if cfg.invert_y {
            v = 1.0 - v;
        }

        if cfg.smoothing > 0.0 {
            let alpha = (1.0 - cfg.smoothing).clamp(0.05, 1.0);
            let (pu, pv) = self.ema.unwrap_or((u, v));
            u = pu + alpha * (u - pu);
            v = pv + alpha * (v - pv);
        }
        self.ema = Some((u, v));

        let mut target = self.region.unwrap_or(bbox);
        if cfg.lock_aspect && target.w > 0.0 && target.h > 0.0 {
            let tablet_ar = match cfg.orientation {
                Orientation::Portrait | Orientation::PortraitFlipped => SENSOR_Y_MAX / SENSOR_X_MAX,
                _ => SENSOR_X_MAX / SENSOR_Y_MAX,
            };
            if tablet_ar > target.w / target.h {
                let h = target.w / tablet_ar;
                target.y += (target.h - h) / 2.0;
                target.h = h;
            } else {
                let w = target.h * tablet_ar;
                target.x += (target.w - w) / 2.0;
                target.w = w;
            }
        }

        let px = target.x + u * target.w;
        let py = target.y + v * target.h;

        let t = self.now();
        let any_held = self.held.iter().any(|&b| b);
        let mut dirty = false;

        if cfg.hover_moves_cursor || contact || any_held {
            let x = (px - bbox.x).clamp(0.0, bbox.w).round() as u32;
            let y = (py - bbox.y).clamp(0.0, bbox.h).round() as u32;
            self.pointer
                .motion_absolute(t, x, y, bbox.w.round() as u32, bbox.h.round() as u32);
            dirty = true;
        }

        let want = if !contact {
            [false; 3]
        } else if s.eraser {
            match cfg.eraser_action {
                EraserAction::None => [false; 3],
                EraserAction::RightClick => [false, true, false],
                EraserAction::MiddleClick => [false, false, true],
            }
        } else {
            [true, false, false]
        };
        for (i, code) in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE].into_iter().enumerate() {
            dirty |= self.set_button(i, code, want[i], t);
        }

        if dirty {
            self.pointer.frame();
        }
    }

    fn set_button(&mut self, idx: usize, code: u32, down: bool, t: u32) -> bool {
        if self.held[idx] == down {
            return false;
        }
        self.held[idx] = down;
        let state = if down {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        self.pointer.button(t, code, state);
        true
    }

    fn release_all(&mut self) {
        let t = self.now();
        let mut dirty = false;
        for (i, code) in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE].into_iter().enumerate() {
            dirty |= self.set_button(i, code, false, t);
        }
        if dirty {
            self.pointer.frame();
        }
        self.ema = None;
    }
}

fn resolve_region(cfg: &Config, bbox: &Region) -> Region {
    match cfg.region {
        RegionSetting::Desktop => *bbox,
        RegionSetting::Fixed(r) => r,
        RegionSetting::ActiveMonitor => config::active_monitor_region().unwrap_or(*bbox),
    }
}

impl Dispatch<WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, 1, qh, ()));
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager = Some(registry.bind::<ZwlrVirtualPointerManagerV1, _, _>(
                        name,
                        version.min(2),
                        qh,
                        (),
                    ));
                }
                "zxdg_output_manager_v1" => {
                    state.xdg_manager = Some(registry.bind::<ZxdgOutputManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    ));
                }
                "wl_output" => {
                    let wl = registry.bind::<WlOutput, _, _>(name, version.min(4), qh, name);
                    state.outputs.insert(
                        name,
                        OutputInfo {
                            wl: Some(wl),
                            ..Default::default()
                        },
                    );
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(info) = state.outputs.remove(&name) {
                    if let Some(xdg) = info.xdg {
                        xdg.destroy();
                    }
                    if let Some(wl) = info.wl {
                        if wl.version() >= 3 {
                            wl.release();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for App {
    fn event(
        _state: &mut Self,
        _output: &WlOutput,
        _event: wayland_client::protocol::wl_output::Event,
        _id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Geometry is taken from xdg-output (logical coordinates).
    }
}

impl Dispatch<ZxdgOutputV1, u32> for App {
    fn event(
        state: &mut Self,
        _xdg: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state.outputs.get_mut(id) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                info.x = x;
                info.y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                info.w = width;
                info.h = height;
            }
            zxdg_output_v1::Event::Name { name } => info.name = name,
            _ => {}
        }
    }
}

delegate_noop!(App: ignore WlSeat);
delegate_noop!(App: ZwlrVirtualPointerManagerV1);
delegate_noop!(App: ZwlrVirtualPointerV1);
delegate_noop!(App: ZxdgOutputManagerV1);
