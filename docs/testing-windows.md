# Testing tigertail on Windows 11

A follow-along checklist for a Windows machine, no tigertail development
environment needed. Tick each box and note the observation; the last section
says what to report back.

## 0. Before you start

- [ ] On the tablet (from any machine with SSH; over USB the address is
  `10.11.99.1`, or use its Wi-Fi address):

  ```
  ssh root@10.11.99.1 systemctl status tigertaild
  ```

  Expect `Active: active (running)` and, in the log tail, `Host enabled the
  HID function` once the tablet is plugged into *some* host. If it is
  inactive: `systemctl start tigertaild`.
- [ ] Config keys that matter for this test live in
  `/home/root/tigertail/tigertail.toml`: `pen = true`, `touch = true`,
  optionally `usb_product = "tigertail"` so the device is easy to spot. After
  editing: `systemctl restart tigertaild`.
- [ ] Note: while tigertaild runs, xochitl ignores pen and touch (they are
  grabbed). `systemctl stop tigertaild` gives them back.

## 1. Enumeration

- [ ] Plug the tablet into the Windows PC with a USB-C data cable. The USB
  network adapter you may already know ("RNDIS") reappears; nothing else
  needs installing.
- [ ] Open **Device Manager** → *Human Interface Devices*. Expect:
  - one **HID-compliant pen**
  - one **HID-compliant touch pad**
  - a **HID-compliant mouse** (the mandatory legacy collection; it never
    sends anything)
  - their parent **USB Input Device** entries
- [ ] Right-click the touch pad → *Properties* → *Details* → *Hardware Ids*.
  Expect `HID\VID_04B3&PID_4010&MI_03` (the pen is `MI_02`; `MI_00/01` are
  the network interfaces). If the vendor/product IDs differ, tigertaild is
  not the device you are looking at.

## 2. Pen

- [ ] **Settings → Bluetooth & devices → Pen & Windows Ink**: the page shows
  pen options (it hides them when no pen is present).
- [ ] Hover the pen ~1 cm above the tablet: a hover cursor moves on screen.
  With the default `fit = "fill"` the whole tablet stretches over the whole
  desktop; with `fit = "contain"` + `aspect_ratio = "16:9"` there are unused
  margins on the tablet; with `cover` the edges of the tablet are cropped.
- [ ] Open **Paint** (or any inking app) and draw: pressure changes stroke
  width (Paint's pencil/brush react to pressure; Windows Ink Workspace
  Sketchpad does too).
- [ ] Tilt: an app with a tilt readout (Krita → Tablet Tester, or Clip
  Studio) shows X/Y tilt changing as the pen leans. Range is ±64° on rM2.
- [ ] Barrel button (the side button on Marker Plus): acts as right-click /
  eraser depending on the app's pen settings.
- [ ] Eraser end (Marker Plus): erases in inking apps.
- [ ] Orientation: with `orientation = "landscape-left"` the tablet's
  button side is on the left; strokes move the right way on screen. Try the
  other values if not.

## 3. Touchpad

- [ ] **Settings → Bluetooth & devices → Touchpad**: the header reads
  **"Your PC has a precision touchpad"**. This is the key check: it means
  Windows loaded its PTP driver for our interface.
- [ ] Move one finger on the tablet: the cursor moves (relative, like a
  laptop touchpad, not to the finger's absolute position).
- [ ] Tap-to-click, two-finger tap (right click).
- [ ] Two-finger scroll in a browser; pinch to zoom in Maps or a photo.
- [ ] Three-finger swipe up (Task View) and three-finger tap (search) —
  whatever the Touchpad settings page has enabled.
- [ ] If the header is missing (page says "Your PC has a touchpad" or shows
  no touchpad section): on the tablet run
  `journalctl -u tigertaild -n 50` and look for
  `Host set touchpad input mode 3 (precision touchpad)`. If it never
  appears, Windows never switched the device to PTP mode — report this
  together with Device Manager's status for the touch pad (*Properties* →
  *General* → *Device status*).

## 4. Palm rejection

- [ ] Rest your hand on the tablet while drawing with the pen: the cursor
  does not jump to the palm and no touchpad clicks happen.
- [ ] Lift the pen; after ~0.5 s (`palm_grace_ms`) touch works again.

## 5. Detach and restore

- [ ] `ssh root@10.11.99.1 systemctl stop tigertaild`: the pen, touch pad and
  mouse entries disappear from Device Manager within a few seconds; the USB
  network adapter stays.
- [ ] Unplug and replug: the device shows up as the stock "reMarkable 2"
  again (no HID devices, no tigertail name).

## 6. Known Windows quirks

- **Cached names.** Windows remembers device names per VID:PID:serial. If
  the name in Device Manager does not follow `usb_product`, open Device
  Manager → *View* → *Show hidden devices*, delete the greyed-out old
  entries, and replug.
- **Certification status.** tigertail answers the PTP certification
  feature report with Microsoft's sample blob. Windows 10/11 do not require
  it; if a future Windows build treats the device as non-certified, PTP
  gestures may be reduced to basic pointer movement.
- **Re-enumeration blips.** Starting or stopping tigertaild re-enumerates
  the whole USB device, so the network adapter drops for a few seconds too.

## 7. What to report back

```
Windows build:            (Settings → System → About, e.g. 22631.xxxx)
Device Manager:           pen [ ] touch pad [ ] mouse [ ]   hardware id MI_03 seen [ ]
Pen & Windows Ink page:   [ ]   hover [ ] pressure [ ] tilt [ ] barrel [ ] eraser [ ]
Touchpad page header:     "Your PC has a precision touchpad" [ ]
Gestures:                 move [ ] tap [ ] 2-finger scroll [ ] pinch [ ] 3-finger [ ]
Palm rejection:           [ ]
Detach restored stock:    [ ]
Tablet log (journalctl -u tigertaild -n 50) attached: [ ]
Screenshot of the Touchpad settings header attached: [ ]
Anything odd:
```
