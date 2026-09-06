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

    /// Grab the pen device exclusively (xochitl stops seeing pen input) [default: true]
    #[arg(long)]
    pub grab_input: bool,

    /// Don't grab the pen device (xochitl will also see input)
    #[arg(long)]
    pub no_grab_input: bool,

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

    /// Path to config file
    #[arg(long, env = "TIGERTAIL_CONFIG")]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Dump decoded pen frames for debugging (no USB gadget involved)
    Dump {
        /// Device to dump: "pen"
        device: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FileConfig {
    pub pen_device: Option<String>,
    pub grab_input: Option<bool>,
    pub orientation: Orientation,
    pub tilt_correction: TiltCorrectionMode,
    pub tilt_correction_gain: Option<f64>,
    pub fit: FitMode,
    pub aspect_ratio: Option<AspectRatio>,
    pub resolution: Option<Resolution>,
}

/// Merged configuration from CLI args and TOML file.
#[derive(Debug, Clone)]
pub struct Config {
    pub pen_device: String,
    pub grab_input: bool,
    pub orientation: Orientation,
    pub tilt_correction: TiltCorrectionMode,
    pub tilt_correction_gain: f64,
    pub fit: FitMode,
    pub aspect_ratio: Option<AspectRatio>,
    pub resolution: Option<Resolution>,
}

impl Config {
    pub fn load(cli: &Cli, device: &DeviceProfile) -> Self {
        let file = cli
            .config
            .as_ref()
            .and_then(|p| load_from_path(p))
            .or_else(load_from_default_paths)
            .unwrap_or_default();

        Self {
            pen_device: cli
                .pen_device
                .clone()
                .unwrap_or_else(|| file.pen_device.unwrap_or(device.pen_device.into())),
            grab_input: if cli.no_grab_input {
                false
            } else {
                cli.grab_input || file.grab_input.unwrap_or(true)
            },
            orientation: cli.orientation.unwrap_or(file.orientation),
            tilt_correction: cli.tilt_correction.unwrap_or(file.tilt_correction),
            tilt_correction_gain: cli
                .tilt_correction_gain
                .or(file.tilt_correction_gain)
                .unwrap_or(1.0),
            fit: cli.fit.unwrap_or(file.fit),
            aspect_ratio: cli.aspect_ratio.or(file.aspect_ratio),
            resolution: cli.resolution.or(file.resolution),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
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

fn load_from_path(path: &Path) -> Option<FileConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&content) {
        Ok(config) => {
            log::debug!("Loaded config from {}", path.display());
            Some(config)
        }
        Err(e) => {
            log::warn!("Failed to parse {}: {}", path.display(), e);
            None
        }
    }
}

fn load_from_default_paths() -> Option<FileConfig> {
    default_config_paths()
        .iter()
        .filter(|p| p.exists())
        .find_map(|p| load_from_path(p))
}

fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("tigertail.toml")];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join(".config").join("tigertail.toml"));
    }
    paths.push(PathBuf::from("/etc/tigertail.toml"));
    paths
}
