//! CLI + TOML configuration, merged the same way as rm-pad's host config:
//! CLI overrides file, file overrides defaults.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::Deserialize;

use rm_pad::device::DeviceProfile;
use rm_pad::display::{AspectRatio, Resolution};
use rm_pad::fit::FitMode;
use rm_pad::orientation::Orientation;
use rm_pad::tilt::TiltCorrectionMode;

#[derive(Parser)]
#[command(name = "tigertaild")]
#[command(about = "Expose this reMarkable as a USB HID pen tablet")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Pen input device path
    #[arg(long)]
    pub pen_device: Option<String>,

    /// Touch input device path
    #[arg(long)]
    pub touch_device: Option<String>,

    /// Don't forward the pen (the pen device may still be grabbed)
    #[arg(long)]
    pub no_pen: bool,

    /// Don't forward touch (the touch device may still be grabbed)
    #[arg(long)]
    pub no_touch: bool,

    /// Grab the pen device exclusively so xochitl stops seeing pen input [default: true]
    #[arg(long, overrides_with = "no_grab_pen")]
    pub grab_pen: bool,

    /// Don't grab the pen device
    #[arg(long)]
    pub no_grab_pen: bool,

    /// Grab the touch device exclusively so xochitl stops seeing touch [default: true]
    #[arg(long, overrides_with = "no_grab_touch")]
    pub grab_touch: bool,

    /// Don't grab the touch device
    #[arg(long)]
    pub no_grab_touch: bool,

    /// Deprecated alias: grab both pen and touch devices
    #[arg(long, hide = true)]
    pub grab_input: bool,

    /// Deprecated alias: grab neither device
    #[arg(long, hide = true)]
    pub no_grab_input: bool,

    /// Disable palm rejection (suppressing touch while the pen is down)
    #[arg(long)]
    pub no_palm_rejection: bool,

    /// Palm rejection grace period after pen-up, in milliseconds
    #[arg(long)]
    pub palm_grace_ms: Option<u64>,

    /// Host screen orientation (portrait, landscape-right, landscape-left, inverted)
    #[arg(long, value_parser = clap::value_parser!(Orientation))]
    pub orientation: Option<Orientation>,

    /// How the pen area fits the target: fill/stretch (default),
    /// contain/fit (letterbox, keep aspect), or cover (crop, keep aspect).
    /// Anything other than fill/stretch requires --aspect-ratio or --resolution.
    #[arg(long, value_parser = clap::value_parser!(FitMode))]
    pub fit: Option<FitMode>,

    /// Target aspect ratio the pen area is fitted into, as WIDTH:HEIGHT (e.g.
    /// 16:9). Mutually exclusive with --resolution.
    #[arg(long, value_parser = clap::value_parser!(AspectRatio), conflicts_with = "resolution")]
    pub aspect_ratio: Option<AspectRatio>,

    /// Target resolution the pen area is fitted into, as WIDTHxHEIGHT (e.g.
    /// 1920x1080). Mutually exclusive with --aspect-ratio.
    #[arg(long, value_parser = clap::value_parser!(Resolution))]
    pub resolution: Option<Resolution>,

    /// Pen tilt-offset correction (off, tilt, tilt-distance)
    #[arg(long, value_parser = clap::value_parser!(TiltCorrectionMode))]
    pub tilt_correction: Option<TiltCorrectionMode>,

    /// Tilt-correction strength: effective lever length in pen digitizer units
    #[arg(long)]
    pub tilt_correction_gain: Option<f64>,

    /// USB product string the host sees (also names the input device and the
    /// USB-ethernet adapter). Stock value is restored on exit.
    #[arg(long)]
    pub usb_product: Option<String>,

    /// USB manufacturer string the host sees. Stock value is restored on exit.
    #[arg(long)]
    pub usb_manufacturer: Option<String>,

    /// Path to config file
    #[arg(long, env = "TIGERTAIL_CONFIG")]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Dump decoded pen or touch frames for debugging (no USB gadget involved)
    Dump {
        /// Device to dump: "pen" or "touch"
        device: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FileConfig {
    pub pen_device: Option<String>,
    pub touch_device: Option<String>,
    pub pen: Option<bool>,
    pub touch: Option<bool>,
    pub grab_pen: Option<bool>,
    pub grab_touch: Option<bool>,
    /// Deprecated alias for both grab_pen and grab_touch.
    pub grab_input: Option<bool>,
    pub palm_rejection: Option<bool>,
    pub palm_grace_ms: Option<u64>,
    pub orientation: Orientation,
    pub tilt_correction: TiltCorrectionMode,
    pub tilt_correction_gain: Option<f64>,
    pub fit: FitMode,
    pub aspect_ratio: Option<AspectRatio>,
    pub resolution: Option<Resolution>,
    pub usb_product: Option<String>,
    pub usb_manufacturer: Option<String>,
}

/// Merged configuration from CLI args and TOML file.
#[derive(Debug, Clone)]
pub struct Config {
    pub pen_device: String,
    pub touch_device: String,
    /// Forward the pen / touch panel to the host. Independent of grabbing.
    pub pen: bool,
    pub touch: bool,
    /// Grab the pen / touch node away from xochitl. Independent of forwarding.
    pub grab_pen: bool,
    pub grab_touch: bool,
    pub palm_rejection: bool,
    pub palm_grace_ms: u64,
    pub orientation: Orientation,
    pub tilt_correction: TiltCorrectionMode,
    pub tilt_correction_gain: f64,
    pub fit: FitMode,
    pub aspect_ratio: Option<AspectRatio>,
    pub resolution: Option<Resolution>,
    /// Gadget-wide USB strings to apply while the HID function is attached.
    pub usb_product: Option<String>,
    pub usb_manufacturer: Option<String>,
}

impl Config {
    pub fn load(cli: &Cli, device: &DeviceProfile) -> Result<Self, String> {
        let file = match cli.config.as_ref() {
            // An explicit config path that exists but does not parse is a
            // hard error: silently running with defaults would hide a broken
            // GUI-written file behind a working-looking pen.
            Some(path) if path.exists() => load_from_path(path)?,
            Some(path) => {
                log::info!("No config at {}; using defaults", path.display());
                FileConfig::default()
            }
            None => load_from_default_paths().unwrap_or_default(),
        };

        Ok(Self {
            pen_device: cli
                .pen_device
                .clone()
                .unwrap_or_else(|| file.pen_device.unwrap_or(device.pen_device.into())),
            touch_device: cli
                .touch_device
                .clone()
                .unwrap_or_else(|| file.touch_device.unwrap_or(device.touch_device.into())),
            pen: !cli.no_pen && file.pen.unwrap_or(true),
            touch: !cli.no_touch && file.touch.unwrap_or(true),
            grab_pen: resolve_grab(
                cli.grab_pen,
                cli.no_grab_pen,
                cli.grab_input,
                cli.no_grab_input,
                file.grab_pen.or(file.grab_input),
            ),
            grab_touch: resolve_grab(
                cli.grab_touch,
                cli.no_grab_touch,
                cli.grab_input,
                cli.no_grab_input,
                file.grab_touch.or(file.grab_input),
            ),
            palm_rejection: !cli.no_palm_rejection && file.palm_rejection.unwrap_or(true),
            palm_grace_ms: cli.palm_grace_ms.or(file.palm_grace_ms).unwrap_or(500),
            orientation: cli.orientation.unwrap_or(file.orientation),
            tilt_correction: cli.tilt_correction.unwrap_or(file.tilt_correction),
            tilt_correction_gain: cli
                .tilt_correction_gain
                .or(file.tilt_correction_gain)
                .unwrap_or(1.0),
            fit: cli.fit.unwrap_or(file.fit),
            aspect_ratio: cli.aspect_ratio.or(file.aspect_ratio),
            resolution: cli.resolution.or(file.resolution),
            usb_product: cli.usb_product.clone().or(file.usb_product),
            usb_manufacturer: cli.usb_manufacturer.clone().or(file.usb_manufacturer),
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.pen && !self.touch {
            return Err("Nothing to forward: both pen and touch are disabled");
        }
        // Aspect ratio and resolution describe the same target two ways; clap
        // rejects both on the CLI, but the file config can still set both.
        if self.aspect_ratio.is_some() && self.resolution.is_some() {
            return Err("Cannot set both aspect_ratio and resolution");
        }
        if self.fit != FitMode::Fill && self.aspect_ratio.is_none() && self.resolution.is_none() {
            return Err("--fit (other than fill) requires either --aspect-ratio or --resolution");
        }
        Ok(())
    }
}

/// Per-device CLI flags win, then the deprecated `--grab-input` pair, then
/// the file (per-device key, falling back to the `grab_input` alias), then
/// the default of grabbing.
fn resolve_grab(
    cli_yes: bool,
    cli_no: bool,
    alias_yes: bool,
    alias_no: bool,
    file: Option<bool>,
) -> bool {
    if cli_yes {
        return true;
    }
    if cli_no {
        return false;
    }
    if alias_yes {
        return true;
    }
    if alias_no {
        return false;
    }
    file.unwrap_or(true)
}

fn load_from_path(path: &Path) -> Result<FileConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("reading {}: {}", path.display(), e))?;
    let config = toml::from_str(&content)
        .map_err(|e| format!("invalid config {}:\n{}", path.display(), e))?;
    log::info!("Loaded config from {}", path.display());
    Ok(config)
}

fn load_from_default_paths() -> Option<FileConfig> {
    default_config_paths()
        .iter()
        .filter(|p| p.exists())
        .find_map(|p| match load_from_path(p) {
            Ok(config) => Some(config),
            Err(e) => {
                log::warn!("{}; trying next location", e);
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped example must parse with every commented key enabled.
    #[test]
    fn example_config_parses_with_all_keys_enabled() {
        let example = include_str!("../../../tigertail.toml.example");
        let enabled: String = example
            .lines()
            .map(|l| {
                l.strip_prefix('#')
                    .filter(|r| r.starts_with(|c: char| c.is_ascii_lowercase()) && r.contains(" = "))
                    .unwrap_or(l)
            })
            .map(|l| format!("{l}\n"))
            .collect();
        let cfg: FileConfig = toml::from_str(&enabled).unwrap_or_else(|e| panic!("{e}"));
        assert!(cfg.aspect_ratio.is_some());
        assert!(cfg.resolution.is_some());
        assert_eq!(cfg.usb_product.as_deref(), Some("tigertail"));
    }
}

fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("tigertail.toml")];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join(".config").join("tigertail.toml"));
    }
    paths.push(PathBuf::from("/etc/tigertail.toml"));
    paths
}
