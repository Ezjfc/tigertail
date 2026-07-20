# Manual test plan

Steps to verify chiral end-to-end once the reMarkable 2 is available. Written
because the tablet could not be connected during development — everything below
the "No hardware needed" section is unverified against real pen input.

## Already verified during development (no hardware)

- `nix develop -c cargo build`, `cargo clippy`, and `cargo test` are clean
  (config TOML round-trip is unit-tested).
- App launches under Hyprland; the Wayland side binds `wlr-virtual-pointer`
  and `xdg-output` without errors.
- Status line reports connection retries against an unreachable tablet.

## 0. UI checks (no hardware needed)

1. Launch `chiral`, open each settings control, change it, quit and relaunch.
   **Expect:** values persist (stored in `~/.config/chiral/config.toml`).
2. "Select region…": all monitors dim, dragging draws a rectangle with a size
   label, Esc cancels, releasing stores the region (label in the settings
   window updates).

## 1. SSH prerequisites

1. Plug the rM2 in over USB. `ping 10.11.99.1` should answer.
2. Install your key: `ssh-copy-id root@10.11.99.1`
   (password: tablet → Settings → Help → Copyrights and licenses → GPLv3
   Compliance, bottom of the page).
3. **Check:** `ssh -o BatchMode=yes root@10.11.99.1 true` exits 0 with no
   password prompt. If this fails, chiral will show "Permission denied" in its
   status line — fix this first.
4. **Check device detection:**

   ```console
   $ ssh root@10.11.99.1 'for d in /sys/class/input/event*; do echo "$d: $(cat $d/device/name)"; done'
   ```

   Expect one line containing `Wacom` (usually `event1`). If the Wacom line is
   missing, note the actual names — the auto-detect grep in `src/pen.rs`
   (`autodetect()`) may need adjusting, or set the device path explicitly in
   Settings → Connection → Device.

## 2. Connection

1. Launch `chiral`. Within a few seconds the status line should read
   "Connected — streaming /dev/input/event1" (or similar).
2. Unplug the USB cable. **Expect:** status becomes "Connection lost …
   reconnecting in 3 s"; no stuck mouse buttons (open a text editor and check
   nothing is being dragged/selected).
3. Replug. **Expect:** reconnects by itself within ~10 s.

## 3. Pen basics (default settings)

Do these with the tablet lying in front of you and a drawing app or just the
desktop visible:

1. Bring the pen tip within ~1 cm of the tablet surface **without touching**.
   **Expect:** the cursor jumps to the corresponding spot on the active monitor
   and follows the hovering pen.
2. Move the pen to each of the 4 corners of the tablet's screen area.
   **Expect:** the cursor reaches the corresponding corners of the mapped
   region (with "Lock aspect ratio" on, there will be unused bands on the
   monitor — that's correct).
3. **Orientation check:** if motion is rotated/mirrored relative to how the
   tablet lies on your desk, cycle Settings → Mapping → Orientation (and, if
   still mirrored, Invert X/Y) until it matches. Note the working combination —
   if the default (`Landscape left`) is wrong for the natural "tablet in front
   of keyboard, landscape" placement, change the default in
   `Config::default()` (`src/config.rs`) and update this file.
4. Touch the tip to the surface and drag. **Expect:** click-and-drag (draws a
   stroke in a paint app, selects text in an editor). Lift → release.
5. Single taps produce single clicks; no double-click on one tap.
6. Flip the marker and touch the eraser end (Marker Plus only).
   **Expect:** right-click (context menu opens).
7. Pull the pen far away from the tablet mid-drag. **Expect:** the drag ends
   (button released), no stuck state.

## 4. Region mapping

1. Settings → "Select region…", drag a rectangle on any monitor.
   **Expect:** pen now maps only within that rectangle; the label in settings
   shows its size and position.
2. "Active monitor": focus a different monitor (if you have several), pull the
   pen away and bring it back. **Expect:** mapping follows the focused monitor
   (region is re-resolved each time the pen comes back into range).
3. "Entire desktop": pen spans all monitors.
4. Toggle "Lock aspect ratio" off. **Expect:** pen area stretches to fill the
   whole region (circles drawn on the tablet become ellipses on screen).

## 5. Behaviour toggles

1. "Hover moves cursor" off. **Expect:** hovering does nothing; the cursor
   only moves/jumps while the tip is pressed down.
2. Pressure threshold ~800. **Expect:** light tip contact no longer clicks;
   pressing firmly does. Set back to 0.
3. Smoothing ~0.7. **Expect:** visibly steadier but laggier strokes. Back
   to 0.
4. Eraser action "No action". **Expect:** eraser hovers/moves but never
   clicks.

## 6. Daemon lifecycle

1. Close the settings window. **Expect:** pen keeps working.
2. Run `chiral` again. **Expect:** no second instance; the existing daemon's
   window re-opens.

## 7. Latency sanity check

Draw fast scribbles in a paint app with smoothing 0. Cursor should track with
no perceptible lag beyond normal mouse latency. If it lags and grows over
seconds, suspect event backlog in the channel (report it — `src/inject.rs`
would need motion coalescing).
