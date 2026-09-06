# tigertail

Native reMarkable 2 daemon that exposes the tablet as a **standard USB HID pen
digitizer plus a precision touchpad**. Plug the tablet into any modern OS
(Windows 11, Linux, macOS) and it works as a drawing tablet with pressure and
tilt, and the touch panel as a multi-finger touchpad — no host-side software.

The stock kernel lacks `usb_f_hid`, so the HID function is implemented in
userspace via **FunctionFS**, attached to the tablet's existing configfs
gadget alongside the USB-ethernet functions (SSH over `10.11.99.1` keeps
working). See `NOTES.md` for the recon findings and design consequences.

The coordinate transforms (orientation, tilt correction, fit modes) are shared
with [rm-pad](https://github.com/Endermanbugzjfc/rm-pad) via its `rm_pad`
library crate.

## Layout

- `crates/tigertaild` — the daemon (armv7 static musl). Reads the Wacom
  digitizer and the touch panel from evdev (grabbing them away from xochitl),
  transforms frames, and streams HID reports on two interfaces: a pen
  digitizer and a Windows Precision Touchpad (PTP).
- `crates/tigertail-ui` — on-device Qt settings UI. Currently a stub behind
  the `qt` feature; see the GUI section of `NOTES.md`.

## Building

Everything runs inside the flake devShell (`nix develop` or direnv):

```bash
cargo build -p tigertaild --release   # target: armv7-unknown-linux-musleabihf
cargo test                            # runs under qemu-arm
nix build .#tigertaild                # reproducible package (static)
```

## Deploying

```bash
./deploy.sh                # scp binary + systemd unit, then:
ssh root@10.11.99.1 systemctl enable --now tigertaild
```

`./recon.sh` dumps the kernel/gadget facts the design depends on.

## Usage

```bash
tigertaild                 # attach HID gadget and forward pen + touch
tigertaild dump pen        # print transformed pen frames, no gadget involved
tigertaild dump touch      # print touch contacts, no gadget involved
tigertaild --help          # all options
```

Forwarding and grabbing are independent per device (unlike rm-pad, where
`--pen-only` also decided what was grabbed):

| key / flag | default | effect |
|---|---|---|
| `pen` / `--no-pen` | on | forward the pen as a HID pen |
| `touch` / `--no-touch` | on | forward touch as a precision touchpad |
| `grab_pen` / `--grab-pen`, `--no-grab-pen` | on | take the pen away from xochitl |
| `grab_touch` / `--grab-touch`, `--no-grab-touch` | on | take touch away from xochitl (e.g. no page turns while drawing, even with `touch = false`) |
| `palm_rejection` / `--no-palm-rejection`, `palm_grace_ms` | on, 500 | suppress touch while the pen is down |

`grab_input` / `--grab-input` remain as a deprecated alias for both grab keys.
Pen mapping options mirror rm-pad: `--orientation`, `--fit contain
--aspect-ratio 16:9`, `--tilt-correction tilt-distance`. `--usb-product` /
`--usb-manufacturer` rename what the host sees (stock strings restored on
exit).

The systemd unit reads the same keys from
**`/home/root/tigertail/tigertail.toml`** (seeded from `tigertail.toml.example`
on first deploy; survives OS updates). That file is the settings store
tigertail-ui will manage; apply edits with `systemctl restart tigertaild`.

On the host the touchpad appears as a hid-multitouch precision touchpad
(libinput gestures, Windows PTP gestures); a small extra "Mouse" HID device
also appears — it is the legacy mouse collection the PTP specification
requires and never sends reports. See `docs/testing-windows.md` for a
Windows test checklist.

Note: attaching/detaching the gadget re-enumerates the USB link, so an SSH
session over USB blips for a few seconds. The daemon restores the stock
gadget on exit (SIGINT/SIGTERM/SIGHUP).
