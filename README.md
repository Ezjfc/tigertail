# tigertail

Native reMarkable 2 daemon that exposes the tablet as a **standard USB HID pen
digitizer**. Plug the tablet into any modern OS (Windows 11, Linux, macOS) and
it works as a drawing tablet with pressure and tilt — no host-side software.

The stock kernel lacks `usb_f_hid`, so the HID function is implemented in
userspace via **FunctionFS**, attached to the tablet's existing configfs
gadget alongside the USB-ethernet functions (SSH over `10.11.99.1` keeps
working). See `NOTES.md` for the recon findings and design consequences.

The coordinate transforms (orientation, tilt correction, fit modes) are shared
with [rm-pad](https://github.com/Endermanbugzjfc/rm-pad) via its `rm_pad`
library crate.

## Layout

- `crates/tigertaild` — the daemon (armv7 static musl). Reads the Wacom
  digitizer from evdev (grabbing it away from xochitl), transforms frames, and
  streams HID reports.
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
tigertaild                 # attach HID gadget and forward the pen
tigertaild dump pen        # print transformed frames, no gadget involved
tigertaild --help          # orientation, fit, tilt-correction options
```

Options mirror rm-pad: `--orientation`, `--fit contain --aspect-ratio 16:9`,
`--tilt-correction tilt-distance`, `--no-grab-input`. The systemd unit reads
the same keys from **`/home/root/tigertail/tigertail.toml`** (seeded from
`tigertail.toml.example` on first deploy; survives OS updates). That file is
the settings store tigertail-ui will manage; apply edits with
`systemctl restart tigertaild`.

Note: attaching/detaching the gadget re-enumerates the USB link, so an SSH
session over USB blips for a few seconds. The daemon restores the stock
gadget on exit (SIGINT/SIGTERM/SIGHUP).
