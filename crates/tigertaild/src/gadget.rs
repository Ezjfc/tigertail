//! USB HID interfaces via FunctionFS, attached to the tablet's existing
//! configfs gadget.
//!
//! The stock kernel has no `usb_f_hid` (see NOTES.md), but it does have
//! FunctionFS, so the HID interfaces are implemented entirely in userspace:
//!
//! 1. create `functions/ffs.tigertail` inside the stock `g_ether` gadget,
//! 2. mount functionfs and feed interface/HID/endpoint descriptors (one
//!    interface + interrupt-IN endpoint per HID interface) to `ep0`,
//! 3. link the function into both configs (`c.1` rndis, `c.2` ecm) so the
//!    interfaces exist whichever configuration the host picks,
//! 4. re-bind the UDC (the USB link re-enumerates; USB-ethernet drops for a
//!    few seconds),
//! 5. answer HID class/control requests (report descriptors, idle, protocol,
//!    feature reports) from the `ep0` thread and stream input reports through
//!    `ep1`, `ep2`, …
//!
//! Teardown restores the stock gadget tree, including on Ctrl-C/SIGTERM.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::os::unix::thread::JoinHandleExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::pen::Result;

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
const HID_REPORT_TYPE_FEATURE: u8 = 3;

/// Signal used to interrupt the blocking `ep0` read at shutdown.
pub const WAKE_SIGNAL: libc::c_int = libc::SIGUSR1;

/// Feature-report handling for one HID interface.
pub trait FeatureReports: Send + Sync {
    /// Bytes to return for GET_REPORT(feature, `report_id`); `None` stalls.
    fn get(&self, report_id: u8) -> Option<Vec<u8>>;
    /// Data received by SET_REPORT(feature, `report_id`), report id first.
    fn set(&self, report_id: u8, data: &[u8]);
}

/// Interfaces without feature reports.
pub struct NoFeatures;

impl FeatureReports for NoFeatures {
    fn get(&self, _: u8) -> Option<Vec<u8>> {
        None
    }
    fn set(&self, _: u8, _: &[u8]) {}
}

/// One HID interface to expose.
pub struct HidInterface {
    pub name: &'static str,
    pub report_descriptor: Vec<u8>,
    /// Largest input report, for the interrupt endpoint packet size.
    pub in_report_len: usize,
    pub features: Box<dyn FeatureReports>,
}

/// Gadget-wide USB descriptor strings to present while attached. `None`
/// keeps the stock value.
#[derive(Debug, Clone, Default)]
pub struct UsbStrings {
    pub product: Option<String>,
    pub manufacturer: Option<String>,
}

/// The 9-byte USB HID class descriptor referencing the report descriptor.
fn hid_class_descriptor(report_desc_len: u16) -> [u8; 9] {
    [
        9, // bLength
        USB_DT_HID,
        0x11, 0x01, // bcdHID 1.11
        0,    // bCountryCode
        1,    // bNumDescriptors
        USB_DT_REPORT,
        report_desc_len as u8,
        (report_desc_len >> 8) as u8,
    ]
}

/// Interface + HID + interrupt-IN endpoint descriptors for every interface
/// at one speed.
fn speed_descriptors(interfaces: &[HidInterface], high_speed: bool) -> Vec<u8> {
    let mut d = Vec::new();
    for (i, iface) in interfaces.iter().enumerate() {
        // Interface descriptor: class HID, no boot protocol. FunctionFS
        // rewrites bInterfaceNumber to whatever slot the composite assigns.
        d.extend_from_slice(&[9, 4, i as u8, 0, 1, 3, 0, 0, 0]);
        d.extend_from_slice(&hid_class_descriptor(iface.report_descriptor.len() as u16));
        // Interrupt IN endpoint; 1 ms polling at both speeds (full-speed
        // frames, high-speed 2^(4-1) microframes).
        let packet = iface.in_report_len.clamp(8, 64) as u8;
        let interval = if high_speed { 4 } else { 1 };
        d.extend_from_slice(&[7, 5, 0x81 + i as u8, 3, packet, 0, interval]);
    }
    d
}

/// FunctionFS v2 descriptor blob (full-speed + high-speed, and forwarding of
/// all interface-recipient control requests so we see HID GET_DESCRIPTOR).
fn descriptors_blob(interfaces: &[HidInterface]) -> Vec<u8> {
    let fs = speed_descriptors(interfaces, false);
    let hs = speed_descriptors(interfaces, true);
    let count = (interfaces.len() * 3) as u32;

    let mut blob = Vec::new();
    blob.extend_from_slice(&FUNCTIONFS_DESCRIPTORS_MAGIC_V2.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes()); // length, patched below
    blob.extend_from_slice(
        &(FUNCTIONFS_HAS_FS_DESC | FUNCTIONFS_HAS_HS_DESC | FUNCTIONFS_ALL_CTRL_RECIP)
            .to_le_bytes(),
    );
    blob.extend_from_slice(&count.to_le_bytes());
    blob.extend_from_slice(&count.to_le_bytes());
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
        libc::mount(src.as_ptr(), target.as_ptr(), fstype.as_ptr(), 0, std::ptr::null())
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

/// The mutated configfs state; `Drop` restores the stock tree.
struct GadgetTree {
    gadget: PathBuf,
    func_dir: PathBuf,
    mnt: PathBuf,
    links: Vec<PathBuf>,
    udc: String,
    /// (attribute path, stock value) for every USB string we overwrote.
    saved_strings: Vec<(PathBuf, String)>,
}

impl GadgetTree {
    /// Perform steps 1-4 (function, functionfs, descriptors, config links,
    /// UDC rebind) and return the tree together with the open ep0.
    fn set_up(interfaces: &[HidInterface], strings: &UsbStrings) -> Result<(Self, File)> {
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
            saved_strings: Vec::new(),
        };

        this.clean_stale();

        fs::create_dir(&this.func_dir)?;
        fs::create_dir_all(&this.mnt)?;
        mount_functionfs(&this.mnt)?;

        let mut ep0 = OpenOptions::new()
            .read(true)
            .write(true)
            .open(this.mnt.join("ep0"))?;
        ep0.write_all(&descriptors_blob(interfaces))?;
        ep0.write_all(&strings_blob())?;
        log::info!(
            "FunctionFS descriptors written ({} HID interface(s): {})",
            interfaces.len(),
            interfaces.iter().map(|i| i.name).collect::<Vec<_>>().join(", ")
        );

        // Config changes need the gadget unbound; re-enumeration drops
        // USB-ethernet for a few seconds.
        log::warn!("Re-binding UDC: the USB link (including SSH over 10.11.99.1) will blip");
        this.write_udc("\n")?;
        this.apply_strings(strings)?;
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

    /// Overwrite the gadget's product/manufacturer strings (while unbound),
    /// remembering the stock values for teardown. The host derives the HID
    /// input device names from these, so this is how the tablet gets renamed.
    fn apply_strings(&mut self, strings: &UsbStrings) -> Result<()> {
        let dir = self.gadget.join("strings/0x409");
        for (attr, value) in [
            ("product", &strings.product),
            ("manufacturer", &strings.manufacturer),
        ] {
            let Some(value) = value else { continue };
            let path = dir.join(attr);
            let stock = fs::read_to_string(&path)?;
            fs::write(&path, value).map_err(|e| format!("writing USB {} string: {}", attr, e))?;
            log::info!("USB {} string: {:?} -> {:?}", attr, stock.trim(), value);
            self.saved_strings.push((path, stock));
        }
        Ok(())
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

impl Drop for GadgetTree {
    fn drop(&mut self) {
        log::info!("Restoring stock gadget tree");
        let _ = self.write_udc("\n");
        for link in &self.links {
            let _ = fs::remove_file(link);
        }
        for (path, stock) in &self.saved_strings {
            if let Err(e) = fs::write(path, stock) {
                log::warn!("Failed to restore {}: {}", path.display(), e);
            }
        }
        if let Err(e) = self.write_udc(&self.udc.clone()) {
            log::warn!("Failed to re-bind stock gadget: {}", e);
        }
        if let Err(e) = umount(&self.mnt) {
            log::warn!("Failed to unmount {}: {}", self.mnt.display(), e);
        }
        let _ = fs::remove_dir(&self.mnt);
        if let Err(e) = fs::remove_dir(&self.func_dir) {
            log::warn!("Failed to remove {}: {}", self.func_dir.display(), e);
        }
    }
}

/// Per-interface state shared with the ep0 control thread.
struct IfaceShared {
    name: &'static str,
    report_descriptor: Vec<u8>,
    last_report: Mutex<Vec<u8>>,
    features: Box<dyn FeatureReports>,
}

struct Shared {
    enabled: AtomicBool,
    ifaces: Vec<IfaceShared>,
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
                    let setup = Setup {
                        bm_request_type: ev[0],
                        b_request: ev[1],
                        w_value: u16::from_le_bytes([ev[2], ev[3]]),
                        w_index: u16::from_le_bytes([ev[4], ev[5]]),
                        w_length: u16::from_le_bytes([ev[6], ev[7]]) as usize,
                    };
                    handle_setup(&mut ep0, &shared, &setup);
                }
                _ => {}
            }
        }
    }
}

struct Setup {
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: usize,
}

fn handle_setup(ep0: &mut File, shared: &Shared, s: &Setup) {
    log::debug!(
        "setup: type={:#04x} req={:#04x} value={:#06x} index={} len={}",
        s.bm_request_type,
        s.b_request,
        s.w_value,
        s.w_index,
        s.w_length
    );

    let send = |ep0: &mut File, data: &[u8]| {
        let n = data.len().min(s.w_length);
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

    // f_fs re-maps wIndex to the function-local interface index.
    let iface = shared.ifaces.get((s.w_index & 0xff) as usize);
    let report_type = (s.w_value >> 8) as u8;
    let report_id = s.w_value as u8;

    if s.bm_request_type & USB_DIR_IN != 0 {
        match (s.bm_request_type & 0x60, s.b_request, iface) {
            // Standard GET_DESCRIPTOR to the interface: HID report descriptor.
            (0x00, USB_REQ_GET_DESCRIPTOR, Some(i)) if report_type == USB_DT_REPORT => {
                send(ep0, &i.report_descriptor);
            }
            (0x00, USB_REQ_GET_DESCRIPTOR, Some(i)) if report_type == USB_DT_HID => {
                send(ep0, &hid_class_descriptor(i.report_descriptor.len() as u16));
            }
            // HID class requests.
            (0x20, HID_REQ_GET_REPORT, Some(i)) if report_type == HID_REPORT_TYPE_FEATURE => {
                match i.features.get(report_id) {
                    Some(data) => send(ep0, &data),
                    None => {
                        log::debug!("{}: unsupported feature report {}", i.name, report_id);
                        stall_in(ep0);
                    }
                }
            }
            (0x20, HID_REQ_GET_REPORT, Some(i)) => {
                let report = i.last_report.lock().unwrap().clone();
                send(ep0, &report);
            }
            (0x20, HID_REQ_GET_IDLE, _) => send(ep0, &[0]),
            (0x20, HID_REQ_GET_PROTOCOL, _) => send(ep0, &[1]),
            _ => stall_in(ep0),
        }
    } else {
        match (s.bm_request_type & 0x60, s.b_request) {
            (0x20, HID_REQ_SET_IDLE) | (0x20, HID_REQ_SET_PROTOCOL) => {
                // No data stage: acknowledge with a zero-length read.
                let _ = ep0.read(&mut []);
            }
            (0x20, HID_REQ_SET_REPORT) => {
                let mut data = vec![0u8; s.w_length];
                let n = ep0.read(&mut data).unwrap_or(0);
                if let Some(i) = iface {
                    if report_type == HID_REPORT_TYPE_FEATURE {
                        i.features.set(report_id, &data[..n]);
                    }
                }
            }
            _ => stall_out(ep0),
        }
    }
}

/// Live gadget: owns the ep0 thread and the configfs mutation.
pub struct Gadget {
    shared: Arc<Shared>,
    ep0_thread: Option<JoinHandle<()>>,
    eps: Vec<Option<File>>,
    // Dropped last: restores the stock tree after ep0 is closed.
    _tree: GadgetTree,
}

impl Gadget {
    /// Attach the interfaces and start serving control requests. The caller
    /// must have a no-op [`WAKE_SIGNAL`] handler installed (without
    /// SA_RESTART) so teardown can interrupt the ep0 read.
    pub fn attach(interfaces: Vec<HidInterface>, strings: &UsbStrings) -> Result<Self> {
        if interfaces.is_empty() {
            return Err("no HID interfaces to expose".into());
        }
        let (tree, ep0) = GadgetTree::set_up(&interfaces, strings)?;

        let mut eps = Vec::with_capacity(interfaces.len());
        for i in 0..interfaces.len() {
            let ep = OpenOptions::new()
                .write(true)
                .open(tree.mnt.join(format!("ep{}", i + 1)))?;
            eps.push(Some(ep));
        }

        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(false),
            ifaces: interfaces
                .into_iter()
                .map(|i| IfaceShared {
                    name: i.name,
                    last_report: Mutex::new(vec![0u8; i.in_report_len]),
                    report_descriptor: i.report_descriptor,
                    features: i.features,
                })
                .collect(),
        });

        let ep0_shared = shared.clone();
        let ep0_thread = thread::Builder::new()
            .name("ep0".into())
            .spawn(move || ep0_loop(ep0, ep0_shared))?;

        log::info!("HID gadget ready; waiting for the host to enable it");
        Ok(Self { shared, ep0_thread: Some(ep0_thread), eps, _tree: tree })
    }

    /// Take the report writer for interface `index` (in attach order).
    pub fn sink(&mut self, index: usize) -> ReportSink {
        let ep = self.eps[index].take().expect("sink taken twice");
        ReportSink { ep, shared: self.shared.clone(), index }
    }
}

impl Drop for Gadget {
    fn drop(&mut self) {
        // ep0 must be closed before the mount goes away, or umount fails
        // with EBUSY and leaves ffs.tigertail behind.
        if let Some(t) = self.ep0_thread.take() {
            unsafe {
                libc::pthread_kill(t.as_pthread_t() as usize as libc::pthread_t, WAKE_SIGNAL);
            }
            let _ = t.join();
        }
        self.eps.clear();
    }
}

/// Writer for one interface's input reports.
pub struct ReportSink {
    ep: File,
    shared: Arc<Shared>,
    index: usize,
}

impl ReportSink {
    /// Record `report` as the interface's current state and, if the host
    /// has enabled the function, send it on the interrupt endpoint.
    pub fn send(&mut self, report: &[u8]) -> Result<()> {
        *self.shared.ifaces[self.index].last_report.lock().unwrap() = report.to_vec();

        if !self.shared.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(e) = self.ep.write_all(report) {
            // The host detaching mid-write surfaces here; the ep0 thread will
            // flip `enabled` off. Not fatal.
            log::debug!("{}: ep write failed: {}", self.shared.ifaces[self.index].name, e);
        }
        Ok(())
    }
}
