use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    Portrait,
    PortraitFlipped,
    LandscapeLeft,
    LandscapeRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EraserAction {
    None,
    RightClick,
    MiddleClick,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum RegionSetting {
    ActiveMonitor,
    Desktop,
    Fixed(Region),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub user: String,
    /// Evdev device path on the tablet; None = auto-detect the Wacom digitizer.
    pub device: Option<String>,
    pub orientation: Orientation,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Move the cursor while the pen hovers (not touching).
    pub hover_moves_cursor: bool,
    /// 0 = trust the tablet's own BTN_TOUCH; >0 = tip is "down" at this raw
    /// pressure (max 4095).
    pub pressure_threshold: i32,
    pub eraser_action: EraserAction,
    /// 0.0 = raw input, towards 1.0 = heavier position smoothing.
    pub smoothing: f64,
    /// Letterbox the tablet's aspect ratio inside the region instead of
    /// stretching to fill it.
    pub lock_aspect: bool,
    pub region: RegionSetting,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "10.11.99.1".into(),
            user: "root".into(),
            device: None,
            orientation: Orientation::LandscapeLeft,
            invert_x: false,
            invert_y: false,
            hover_moves_cursor: true,
            pressure_threshold: 0,
            eraser_action: EraserAction::RightClick,
            smoothing: 0.0,
            lock_aspect: true,
            region: RegionSetting::ActiveMonitor,
        }
    }
}

impl Config {
    fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        base.join("chiral").join("config.toml")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("chiral: invalid config, using defaults: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Logical rect of the focused monitor, in global compositor coordinates.
/// Hyprland-specific; returns None elsewhere so callers can fall back to the
/// whole layout.
pub fn active_monitor_region() -> Option<Region> {
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let monitors: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for m in monitors.as_array()? {
        if m["focused"].as_bool() != Some(true) {
            continue;
        }
        let scale = m["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
        let mut w = m["width"].as_f64()? / scale;
        let mut h = m["height"].as_f64()? / scale;
        if m["transform"].as_i64().unwrap_or(0) % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        return Some(Region {
            x: m["x"].as_f64()?,
            y: m["y"].as_f64()?,
            w,
            h,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip_default() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.region, cfg.region);
        assert_eq!(back.orientation, cfg.orientation);
        assert_eq!(back.host, cfg.host);
    }

    #[test]
    fn toml_round_trip_fixed_region() {
        let cfg = Config {
            region: RegionSetting::Fixed(Region {
                x: 100.0,
                y: 50.0,
                w: 800.0,
                h: 600.0,
            }),
            device: Some("/dev/input/event1".into()),
            pressure_threshold: 600,
            ..Config::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.region, cfg.region);
        assert_eq!(back.device, cfg.device);
        assert_eq!(back.pressure_threshold, cfg.pressure_threshold);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let back: Config = toml::from_str("host = \"192.168.1.20\"\n").unwrap();
        assert_eq!(back.host, "192.168.1.20");
        assert_eq!(back.region, RegionSetting::ActiveMonitor);
        assert!(back.hover_moves_cursor);
    }
}
