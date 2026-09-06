# Device recon findings (reMarkable 2, 2026-09-06)

Collected with `./recon.sh` against `root@10.11.99.1` (key `~/.ssh/id_ed25519.remarkable`).

## Device
- Model: `reMarkable 2.0`, kernel `5.4.70-v1.6.3-rm11x` (armv7l), BusyBox userland, systemd present.
- Pen: `Wacom I2C Digitizer` (bus 0x18, vendor 0x2d1f) on `/dev/input/event1`.
- Touch: `pt_mt` on `/dev/input/event2`.
- UDC: `ci_hdrc.0`.

## USB gadget
- The stock USB ethernet is **already a configfs composite gadget**:
  `/sys/kernel/config/usb_gadget/g_ether` (set up by the `g_ether.ko` in
  `gadget/legacy` — name notwithstanding, the configfs tree is live) with:
  - config `c.1`: `rndis.usb0` (+ `os_desc` so Windows picks it)
  - config `c.2`: `ecm.usb1`
- Function types accepted by this kernel (probed via reversible
  `mkdir functions/<type>.probe`):
  - `ffs` **YES** ← our vector
  - `acm` YES, `ncm` YES, `mass_storage` YES
  - `hid` **NO** (`usb_f_hid` neither built-in nor a module)
  - `midi` no

## Consequence for the HID design
- The planned `f_hid` + `/dev/hidg0` path is unavailable without building a
  kernel module for `5.4.70-v1.6.3-rm11x` (possible later optimization).
- Instead tigertaild implements the HID function in userspace via
  **FunctionFS**: create `functions/ffs.tigertail`, mount functionfs, write
  USB interface/HID-class/endpoint descriptors to `ep0`, answer HID class
  control requests (report-descriptor GET_DESCRIPTOR, GET/SET_IDLE,
  GET/SET_PROTOCOL) from the ep0 event loop, stream input reports through the
  interrupt-IN endpoint file.
- The function must be linked into **both** configs (`c.1` and `c.2`) so the
  pen exists whichever configuration the host selects.
- Attaching the function requires a brief UDC unbind/rebind
  (`echo '' > UDC; …; echo ci_hdrc.0 > UDC`): the USB link re-enumerates, so
  USB-ethernet SSH sessions drop for a few seconds. Do the whole
  mutation in one on-device process and restore the original tree on exit.

## End-to-end result (2026-09-06)

Verified live with this dev machine as the USB host:
- tigertaild attaches the FunctionFS HID function, the host re-enumerates
  `04b3:4010` with a third (HID) interface in both configs, reads our 93-byte
  report descriptor over ep0, and creates input device "reMarkable 2"
  (`INPUT_PROP_DIRECT`). USB-ethernet SSH keeps working alongside.
- SIGTERM teardown restores the stock two-function gadget and the HID
  interface disappears from the host.
- Pen motion/pressure still needs a human draw test (nobody at the tablet).

## GUI (tigertail-ui) status

Scaffolded behind a `qt` cargo feature (stub otherwise). Blockers charted for
the real build, in order of hardness:
1. **Cross Qt**: `pkgsCross.armv7l-hf-multiplatform.libsForQt5.qtbase` is
   5.15.19 in nixpkgs 26.05 (matches device Qt 5.15 minor), but a dry-run
   shows ~740 derivations built from source — hours of compile. Do it once,
   cache it.
2. **glibc symbols**: nixpkgs cross glibc is far newer than the tablet's;
   a dynamically linked GUI will demand symbol versions the device lacks.
   Fallbacks: `cargo-zigbuild` with a pinned glibc version, or the official
   reMarkable OE SDK toolchain fetched by hash.
3. **Display stack**: the rM2 has no kernel framebuffer — xochitl drives the
   panel directly. Third-party GUI apps conventionally need
   remarkable2-framebuffer (rm2fb) and an e-paper-aware Qt platform plugin.
   Investigate what Toltec's Qt apps (e.g. Oxide) ship and mirror that.

## Touch as a Windows Precision Touchpad (2026-09-06)

- Second HID interface in the same FunctionFS function (f_fs re-maps
  interface numbers and hands `wIndex` to userspace as the function-local
  index, so ep0 dispatches per interface; ep files are `ep1` pen, `ep2`
  touchpad, in descriptor order).
- Descriptor follows Microsoft's PTP sample: Touch Pad TLC (report ID 1: 5
  fingers × confidence/tip/contact-id/X/Y with mm physical size, scan time in
  100 µs, contact count, button 1; feature ID 2 contact-count max + pad type;
  feature ID 3 the 256-byte certification blob; feature ID 6 latency mode),
  Configuration TLC (feature ID 4 input mode, ID 5 surface/button switch), and
  the legacy Mouse TLC (report ID 7, never sent).
- Feature reports are answered/stored in userspace (`hid/touchpad.rs`
  `Features`). Linux hid-multitouch sets input mode 3 at probe; Windows does
  the same when its PTP driver binds.
- Verified on a Linux host: `PROP=5` (POINTER|BUTTONPAD) touchpad bound by
  hid-multitouch, listed by Hyprland as `remarkable-<product>-touchpad`.
- Teardown is deterministic: the ep0 thread is woken with SIGUSR1 and joined
  before the functionfs mount is unmounted, otherwise `umount` returns EBUSY
  and `ffs.tigertail` lingers.

### Host semantics that bit us (fixed 2026-09-07)

- **Contact ids are per touch, not per slot.** The panel driver reuses slot
  0 for the next finger and may deliver one finger's release and the next
  finger's press in the same evdev frame; with id = slot the host saw one
  contact teleport (libinput: motion, tap cancelled).
- **A lifted contact must be reported once with tip = 0.** Windows-8-class
  devices in hid-multitouch (`MT_CLS_WIN_8`) have no `INPUT_MT_DROP_UNUSED`;
  a contact that just vanishes from the report stays down until the 100 ms
  sticky-finger timer. Symptoms: laggy taps, rapid taps becoming two-finger
  gestures, two-finger scroll broken after any momentary lift.
- **Scan time must change between reports**, or hid-multitouch treats the
  report as a hybrid-mode continuation packet and ignores its contact count.
- Active contacts are packed first; hosts only look at the first
  `contact count` finger collections of a parallel-mode report.
