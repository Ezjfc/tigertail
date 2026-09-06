//! USB HID pen function via FunctionFS, attached to the tablet's existing
//! configfs gadget.
//!
//! The stock kernel has no `usb_f_hid` (see NOTES.md), but it does have
//! FunctionFS, so the HID interface is implemented entirely in userspace:
//!
//! 1. create `functions/ffs.tigertail` inside the stock `g_ether` gadget,
//! 2. mount functionfs and feed interface/HID/endpoint descriptors to `ep0`,
//! 3. link the function into both configs (`c.1` rndis, `c.2` ecm) so the pen
//!    exists whichever configuration the host picks,
//! 4. re-bind the UDC (the USB link re-enumerates; USB-ethernet drops for a
//!    few seconds),
//! 5. answer HID class/control requests (report descriptor, idle, protocol)
//!    from the `ep0` event thread and stream input reports through `ep1`.
//!
//! Teardown restores the stock gadget tree, including on Ctrl-C/SIGTERM.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::hid;
use crate::pen::{PenFrame, PenGeometry, PenSink, Result};

const GADGET_DIR: &str = "/sys/kernel/config/usb_gadget/g_ether";
const INSTANCE: &str = "tigertail";
const MOUNT_DIR: &str = "/dev/ffs-tigertail";

// linux/usb/functionfs.h
const FUNCTIONFS_STRINGS_MAGIC: u32 = 2;
const FUNCTIONFS_DESCRIPTORS_MAGIC_V2: u32 = 3;
const FUNCTIONFS_HAS_FS_DESC: u32 = 1;
const FUNCTIONFS_HAS_HS_DESC: u32 = 2;
const FUNCTIONFS_ALL_CTRL_RECIP: u32 = 64;

const EVENT_SIZE: usize = 12;
const FUNCTIONFS_ENABLE: u8 = 2;
const FUNCTIONFS_DISABLE: u8 = 3;
const FUNCTIONFS_SETUP: u8 = 4;

// USB / HID control requests.
const USB_DIR_IN: u8 = 0x80;
const USB_REQ_GET_DESCRIPTOR: u8 = 6;
const USB_DT_HID: u8 = 0x21;
const USB_DT_REPORT: u8 = 0x22;
const HID_REQ_GET_REPORT: u8 = 0x01;
const HID_REQ_GET_IDLE: u8 = 0x02;
const HID_REQ_GET_PROTOCOL: u8 = 0x03;
const HID_REQ_SET_REPORT: u8 = 0x09;
const HID_REQ_SET_IDLE: u8 = 0x0A;
const HID_REQ_SET_PROTOCOL: u8 = 0x0B;

/// The 9-byte USB HID class descriptor referencing the report descriptor.
fn hid_class_descriptor(report_desc_len: u16) -> [u8; 9] {
    [
        9,    // bLength
        USB_DT_HID,
        0x11, 0x01, // bcdHID 1.11
        0,    // bCountryCode
        1,    // bNumDescriptors
        USB_DT_REPORT,
        report_desc_len as u8,
        (report_desc_len >> 8) as u8,
    ]
}

/// Interface + HID + interrupt-IN endpoint descriptors for one speed.
fn speed_descriptors(report_desc_len: u16, high_speed: bool) -> Vec<u8> {
    let mut d = Vec::new();
    // Interface descriptor: class HID, no boot protocol. FunctionFS rewrites
    // bInterfaceNumber to whatever slot the composite assigns.
    d.extend_from_slice(&[9, 4, 0, 0, 1, 3, 0, 0, 0]);
    d.extend_from_slice(&hid_class_descriptor(report_desc_len));
    // Endpoint: interrupt IN. 16-byte packets fit the 9-byte report; 1 ms
    // polling both at full speed (frames) and high speed (2^(4-1) microframes).
    let interval = if high_speed { 4 } else { 1 };
    d.extend_from_slice(&[7, 5, 0x81, 3, 16, 0, interval]);
    d
}

/// FunctionFS v2 descriptor blob (full-speed + high-speed, and forwarding of
/// all interface-recipient control requests so we see the HID GET_DESCRIPTOR).
fn descriptors_blob(report_desc_len: u16) -> Vec<u8> {
    let fs = speed_descriptors(report_desc_len, false);
    let hs = speed_descriptors(report_desc_len, true);

    let mut blob = Vec::new();
    blob.extend_from_slice(&FUNCTIONFS_DESCRIPTORS_MAGIC_V2.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes()); // length, patched below
    blob.extend_from_slice(
        &(FUNCTIONFS_HAS_FS_DESC | FUNCTIONFS_HAS_HS_DESC | FUNCTIONFS_ALL_CTRL_RECIP)
            .to_le_bytes(),
    );
    // One descriptor count per HAS_*_DESC flag: 3 descriptors each.
    blob.extend_from_slice(&3u32.to_le_bytes());
    blob.extend_from_slice(&3u32.to_le_bytes());
    blob.extend_from_slice(&fs);
    blob.extend_from_slice(&hs);

    let len = blob.len() as u32;
    blob[4..8].copy_from_slice(&len.to_le_bytes());
    blob
}

/// FunctionFS strings blob: no strings, no languages.
fn strings_blob() -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend_from_slice(&FUNCTIONFS_STRINGS_MAGIC.to_le_bytes());
    blob.extend_from_slice(&16u32.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes()); // str_count
    blob.extend_from_slice(&0u32.to_le_bytes()); // lang_count
    blob
}

fn mount_functionfs(mnt: &Path) -> std::io::Result<()> {
    let src = std::ffi::CString::new(INSTANCE).unwrap();
    let target = std::ffi::CString::new(mnt.as_os_str().as_bytes()).unwrap();
    let fstype = std::ffi::CString::new("functionfs").unwrap();
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn umount(mnt: &Path) -> std::io::Result<()> {
    let target = std::ffi::CString::new(mnt.as_os_str().as_bytes()).unwrap();
    let rc = unsafe { libc::umount(target.as_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The mutated gadget state; `Drop` restores the stock tree.
struct HidGadget {
    gadget: PathBuf,
    func_dir: PathBuf,
    mnt: PathBuf,
    links: Vec<PathBuf>,
    udc: String,
}

impl HidGadget {
    /// Perform steps 1-4 (function, functionfs, descriptors, config links,
    /// UDC rebind) and return the gadget together with the open ep0.
    fn set_up(report_desc_len: u16) -> Result<(Self, File)> {
        let gadget = PathBuf::from(GADGET_DIR);
        if !gadget.is_dir() {
            return Err(format!("gadget dir {} not found", gadget.display()).into());
        }

        // The UDC to rebind: whatever the gadget is bound to now, or the
        // first (only) controller if it is currently unbound.
        let mut udc = fs::read_to_string(gadget.join("UDC"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if udc.is_empty() {
            let entry = fs::read_dir("/sys/class/udc")?
                .next()
                .ok_or("no UDC in /sys/class/udc")??;
            udc = entry.file_name().to_string_lossy().into_owned();
        }
        log::info!("Using UDC {}", udc);

        let mut this = Self {
            func_dir: gadget.join(format!("functions/ffs.{}", INSTANCE)),
            mnt: PathBuf::from(MOUNT_DIR),
            links: Vec::new(),
            udc,
            gadget,
        };

        this.clean_stale();

        fs::create_dir(&this.func_dir)?;
        fs::create_dir_all(&this.mnt)?;
        mount_functionfs(&this.mnt)?;

        let mut ep0 = OpenOptions::new()
            .read(true)
            .write(true)
            .open(this.mnt.join("ep0"))?;
        ep0.write_all(&descriptors_blob(report_desc_len))?;
        ep0.write_all(&strings_blob())?;
        log::info!("FunctionFS descriptors written");

        // Config changes need the gadget unbound; re-enumeration drops
        // USB-ethernet for a few seconds.
        log::warn!("Re-binding UDC: the USB link (including SSH over 10.11.99.1) will blip");
        this.write_udc("\n")?;
        for config in ["configs/c.1", "configs/c.2"] {
            let link = this.gadget.join(config).join(format!("ffs.{}", INSTANCE));
            symlink(&this.func_dir, &link)?;
            this.links.push(link);
        }
        this.write_udc(&this.udc.clone())?;
        log::info!("Gadget re-bound with HID function attached");

        Ok((this, ep0))
    }

    fn write_udc(&self, value: &str) -> Result<()> {
        fs::write(self.gadget.join("UDC"), value)
            .map_err(|e| format!("writing UDC {:?}: {}", value.trim(), e).into())
    }

    /// Best-effort removal of leftovers from a crashed previous run.
    fn clean_stale(&mut self) {
        if self.mnt.join("ep0").exists() {
            let _ = umount(&self.mnt);
        }
        let stale_links: Vec<_> = ["configs/c.1", "configs/c.2"]
            .iter()
            .map(|c| self.gadget.join(c).join(format!("ffs.{}", INSTANCE)))
            .filter(|l| l.symlink_metadata().is_ok())
            .collect();
        if !stale_links.is_empty() {
            let _ = self.write_udc("\n");
            for link in stale_links {
                let _ = fs::remove_file(link);
            }
            let _ = self.write_udc(&self.udc.clone());
        }
        if self.func_dir.exists() {
            let _ = fs::remove_dir(&self.func_dir);
        }
    }
}

impl Drop for HidGadget {
    fn drop(&mut self) {
        log::info!("Restoring stock gadget tree");
        let _ = self.write_udc("\n");
        for link in &self.links {
            let _ = fs::remove_file(link);
        }
        if let Err(e) = self.write_udc(&self.udc.clone()) {
            log::warn!("Failed to re-bind stock gadget: {}", e);
        }
        let _ = umount(&self.mnt);
        let _ = fs::remove_dir(&self.mnt);
        let _ = fs::remove_dir(&self.func_dir);
    }
}

/// State shared between the pen loop and the ep0 control thread.
struct Shared {
    enabled: AtomicBool,
    last_report: Mutex<[u8; hid::REPORT_LEN]>,
    report_descriptor: Vec<u8>,
}

/// Serve ep0: enable/disable tracking and HID control requests.
fn ep0_loop(mut ep0: File, shared: Arc<Shared>) {
    let mut buf = [0u8; EVENT_SIZE * 8];
    loop {
        let n = match ep0.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => {
                log::debug!("ep0 read ended: {}", e);
                return;
            }
        };

        for ev in buf[..n].chunks_exact(EVENT_SIZE) {
            match ev[8] {
                FUNCTIONFS_ENABLE => {
                    log::info!("Host enabled the HID function");
                    shared.enabled.store(true, Ordering::Release);
                }
                FUNCTIONFS_DISABLE => {
                    log::info!("Host disabled the HID function");
                    shared.enabled.store(false, Ordering::Release);
                }
                FUNCTIONFS_SETUP => {
                    let bm_request_type = ev[0];
                    let b_request = ev[1];
                    let w_value = u16::from_le_bytes([ev[2], ev[3]]);
                    let w_length = u16::from_le_bytes([ev[6], ev[7]]) as usize;
                    handle_setup(
                        &mut ep0,
                        &shared,
                        bm_request_type,
                        b_request,
                        w_value,
                        w_length,
                    );
                }
                _ => {}
            }
        }
    }
}

fn handle_setup(
    ep0: &mut File,
    shared: &Shared,
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_length: usize,
) {
    log::debug!(
        "setup: type={:#04x} req={:#04x} value={:#06x} len={}",
        bm_request_type,
        b_request,
        w_value,
        w_length
    );

    let send = |ep0: &mut File, data: &[u8]| {
        let n = data.len().min(w_length);
        if let Err(e) = ep0.write_all(&data[..n]) {
            log::debug!("ep0 reply failed: {}", e);
        }
    };
    // I/O in the wrong direction stalls the request — FunctionFS's way of
    // saying "unsupported".
    let stall_in = |ep0: &mut File| {
        let _ = ep0.read(&mut []);
    };
    let stall_out = |ep0: &mut File| {
        let _ = ep0.write(&[]);
    };

    if bm_request_type & USB_DIR_IN != 0 {
        match (bm_request_type & 0x60, b_request) {
            // Standard GET_DESCRIPTOR to the interface: HID report descriptor.
            (0x00, USB_REQ_GET_DESCRIPTOR) if (w_value >> 8) as u8 == USB_DT_REPORT => {
                send(ep0, &shared.report_descriptor.clone());
            }
            (0x00, USB_REQ_GET_DESCRIPTOR) if (w_value >> 8) as u8 == USB_DT_HID => {
                send(
                    ep0,
                    &hid_class_descriptor(shared.report_descriptor.len() as u16),
                );
            }
            // HID class requests.
            (0x20, HID_REQ_GET_REPORT) => {
                let report = *shared.last_report.lock().unwrap();
                send(ep0, &report);
            }
            (0x20, HID_REQ_GET_IDLE) => send(ep0, &[0]),
            (0x20, HID_REQ_GET_PROTOCOL) => send(ep0, &[1]),
            _ => stall_in(ep0),
        }
    } else {
        match (bm_request_type & 0x60, b_request) {
            (0x20, HID_REQ_SET_IDLE) | (0x20, HID_REQ_SET_PROTOCOL) => {
                // No data stage: acknowledge with a zero-length read.
                let _ = ep0.read(&mut []);
            }
            (0x20, HID_REQ_SET_REPORT) => {
                // Read and discard the (unused) output report data.
                let mut data = vec![0u8; w_length];
                let _ = ep0.read(&mut data);
            }
            _ => stall_out(ep0),
        }
    }
}

/// [`PenSink`] that turns pen frames into HID input reports on ep1.
pub struct HidSink {
    state: Option<Active>,
}

struct Active {
    // Field order matters: close ep1 and join ep0 concerns before the gadget
    // teardown in `HidGadget::drop` restores the stock tree.
    ep1: File,
    shared: Arc<Shared>,
    geo: PenGeometry,
    _gadget: HidGadget,
}

impl HidSink {
    pub fn new() -> Self {
        Self { state: None }
    }
}

impl PenSink for HidSink {
    fn begin(&mut self, geometry: &PenGeometry) -> Result<()> {
        let report_descriptor = hid::report_descriptor(geometry);
        let (gadget, ep0) = HidGadget::set_up(report_descriptor.len() as u16)?;

        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(false),
            last_report: Mutex::new([0u8; hid::REPORT_LEN]),
            report_descriptor,
        });

        let ep1 = OpenOptions::new()
            .write(true)
            .open(gadget.mnt.join("ep1"))?;

        let ep0_shared = shared.clone();
        thread::Builder::new()
            .name("ep0".into())
            .spawn(move || ep0_loop(ep0, ep0_shared))?;

        self.state = Some(Active {
            ep1,
            shared,
            geo: *geometry,
            _gadget: gadget,
        });
        log::info!("HID pen gadget ready; waiting for the host to enable it");
        Ok(())
    }

    fn frame(&mut self, frame: &PenFrame) -> Result<()> {
        let Some(active) = self.state.as_mut() else {
            return Ok(());
        };

        let report = hid::pack_report(frame, &active.geo);
        *active.shared.last_report.lock().unwrap() = report;

        if !active.shared.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(e) = active.ep1.write_all(&report) {
            // The host detaching mid-write surfaces here; the ep0 thread will
            // flip `enabled` off. Not fatal.
            log::debug!("ep1 write failed: {}", e);
        }
        Ok(())
    }
}
