# chiral

Use a **reMarkable 2** as a drawing tablet on Wayland. Not screen sharing —
the reverse: pen input on the tablet drives your desktop cursor.

- Hover the pen near the tablet surface → the cursor jumps to the mapped spot.
- Tip down → mouse down; lift → mouse up. Eraser end → right click (configurable).
- Pen input maps to a screen region: the whole active monitor by default, or a
  rectangle you drag out screenshot-style on any monitor.

## How it works

No software is installed on the tablet. chiral reads the Wacom digitizer's raw
evdev stream over SSH (`cat /dev/input/eventN`) and injects pointer events
through the `wlr-virtual-pointer-unstable-v1` Wayland protocol — so it needs no
root, no uinput, and no udev rules on the host.

Requirements:

- A compositor implementing `wlr-virtual-pointer` and `xdg-output`:
  **Hyprland** (first-class; the "active monitor" region uses `hyprctl`),
  Sway, and other wlroots compositors.
- SSH key access to the tablet (see below).

## Setup

### 1. SSH access to the tablet

Connect the rM2 over USB (it appears as `10.11.99.1`) and install your key.
The root password is shown on the tablet under **Settings → Help → Copyrights
and licenses → GPLv3 Compliance**:

```console
$ ssh-copy-id root@10.11.99.1
```

Verify with `ssh root@10.11.99.1 true` — it must succeed without a password
prompt (chiral runs ssh in BatchMode).

### 2. Run

```console
$ nix run github:endermanbugzjfc/chiral   # or `nix run .` from a checkout
```

chiral is a background daemon: closing the settings window keeps the bridge
running; launching `chiral` again re-opens the window of the running instance.

### NixOS module

```nix
{
  inputs.chiral.url = "github:endermanbugzjfc/chiral";
  # ...
  imports = [ chiral.nixosModules.default ];
  programs.chiral.enable = true;
}
```

The module installs the package and adds a system-wide `Host remarkable` ssh
alias for the tablet (options: `programs.chiral.remarkableHost`,
`.sshHostAlias`, `.package`). No other system configuration is needed.

## Settings

Everything lives in the settings window (persisted to
`~/.config/chiral/config.toml`):

| Setting | Meaning |
| --- | --- |
| Mapping region | Active monitor (default), entire desktop, or a dragged-out rectangle |
| Hover moves cursor | Whether the cursor follows the pen while hovering (off = only while touching) |
| Pressure threshold | Raw pressure (0–4095) required to click; 0 uses the tablet's own tip detection |
| Eraser tool | Right click / middle click / nothing when the Marker Plus eraser touches |
| Smoothing | Exponential position smoothing; trade jitter for latency |
| Orientation + Invert X/Y | How the tablet lies on your desk. If motion comes out rotated or mirrored, cycle the orientation and inverts until it matches |
| Lock aspect ratio | Letterbox the tablet's 4:3 active area inside the region instead of stretching |
| Connection | Tablet host/user, evdev device override (blank = auto-detect the Wacom device), Reconnect |

## Development

```console
$ nix develop
$ cargo build
```

Manual hardware verification steps live in [TESTING.md](TESTING.md).
