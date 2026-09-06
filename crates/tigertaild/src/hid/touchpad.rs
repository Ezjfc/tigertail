//! Windows Precision Touchpad (PTP) interface: report descriptor, input
//! report packing, and the feature reports the host negotiates.
//!
//! Layout follows Microsoft's sample PTP descriptor (touch pad TLC with a
//! parallel-mode finger array, a configuration TLC, and a legacy mouse TLC).
//! Report IDs are scoped to this interface; the pen interface has none.
//!
//! Input report (ID 1, 35 bytes):
//!
//! ```text
//! byte 0        report id (1)
//! bytes 1..31   5 × { bit0 confidence, bit1 tip switch, pad; contact id;
//!                     X lo, hi; Y lo, hi }
//! bytes 31..33  scan time, 100 µs units
//! byte 33       contact count
//! byte 34       bit0 button 1 (never set: tap-to-click is host-side)
//! ```

use std::sync::Mutex;

use crate::gadget::FeatureReports;
use crate::touch::{TouchFrame, TouchGeometry, MAX_CONTACTS};

pub const REPORT_ID_TOUCHPAD: u8 = 1;
pub const REPORT_ID_MAX_COUNT: u8 = 2;
pub const REPORT_ID_PTPHQA: u8 = 3;
pub const REPORT_ID_INPUT_MODE: u8 = 4;
pub const REPORT_ID_FUNCTION_SWITCH: u8 = 5;
pub const REPORT_ID_LATENCY: u8 = 6;
pub const REPORT_ID_MOUSE: u8 = 7;

const CONTACT_LEN: usize = 6;
pub const INPUT_REPORT_LEN: usize = 1 + MAX_CONTACTS * CONTACT_LEN + 2 + 1 + 1;

/// Input Mode value the host writes to switch from mouse to touchpad reports.
pub const INPUT_MODE_TOUCHPAD: u8 = 3;

/// Build the report descriptor for the given (oriented) touch geometry.
pub fn report_descriptor(geo: &TouchGeometry) -> Vec<u8> {
    let x_max = geo.x_max as u16;
    let y_max = geo.y_max as u16;
    let res = geo.resolution.max(1);
    let x_mm = (geo.x_max / res).max(1) as u16;
    let y_mm = (geo.y_max / res).max(1) as u16;

    let mut d = vec![
        0x05, 0x0D, // Usage Page (Digitizers)
        0x09, 0x05, // Usage (Touch Pad)
        0xA1, 0x01, // Collection (Application)
        0x85, REPORT_ID_TOUCHPAD,
    ];
    for _ in 0..MAX_CONTACTS {
        d.extend_from_slice(&[
            0x09, 0x22, //   Usage (Finger)
            0xA1, 0x02, //   Collection (Logical)
            0x15, 0x00, //     Logical Minimum (0)
            0x25, 0x01, //     Logical Maximum (1)
            0x09, 0x47, //     Usage (Confidence)
            0x09, 0x42, //     Usage (Tip Switch)
            0x95, 0x02, //     Report Count (2)
            0x75, 0x01, //     Report Size (1)
            0x81, 0x02, //     Input (Data,Var,Abs)
            0x95, 0x01, //     Report Count (1)
            0x75, 0x06, //     Report Size (6)
            0x81, 0x03, //     Input (Const) — pad
            0x25, (MAX_CONTACTS - 1) as u8, // Logical Maximum
            0x09, 0x51, //     Usage (Contact Identifier)
            0x75, 0x08, //     Report Size (8)
            0x81, 0x02, //     Input (Data,Var,Abs)
            0x05, 0x01, //     Usage Page (Generic Desktop)
            0x15, 0x00, //     Logical Minimum (0)
            0x26, x_max as u8, (x_max >> 8) as u8, // Logical Maximum
            0x75, 0x10, //     Report Size (16)
            0x55, 0x0F, //     Unit Exponent (-1)
            0x65, 0x11, //     Unit (SI linear: cm) → mm
            0x09, 0x30, //     Usage (X)
            0x35, 0x00, //     Physical Minimum (0)
            0x46, x_mm as u8, (x_mm >> 8) as u8, // Physical Maximum
            0x95, 0x01, //     Report Count (1)
            0x81, 0x02, //     Input (Data,Var,Abs)
            0x26, y_max as u8, (y_max >> 8) as u8, // Logical Maximum
            0x46, y_mm as u8, (y_mm >> 8) as u8, // Physical Maximum
            0x09, 0x31, //     Usage (Y)
            0x81, 0x02, //     Input (Data,Var,Abs)
            0xC0, //         End Collection
        ]);
    }
    d.extend_from_slice(&[
        0x55, 0x0C, //   Unit Exponent (-4)
        0x66, 0x01, 0x10, // Unit (SI linear: seconds) → 100 µs
        0x47, 0xFF, 0xFF, 0x00, 0x00, // Physical Maximum (65535)
        0x27, 0xFF, 0xFF, 0x00, 0x00, // Logical Maximum (65535)
        0x75, 0x10, //   Report Size (16)
        0x95, 0x01, //   Report Count (1)
        0x05, 0x0D, //   Usage Page (Digitizers)
        0x09, 0x56, //   Usage (Scan Time)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0x55, 0x00, //   Unit Exponent (0)
        0x65, 0x00, //   Unit (none)
        0x45, 0x00, //   Physical Maximum (0)
        0x09, 0x54, //   Usage (Contact Count)
        0x25, 0x7F, //   Logical Maximum (127)
        0x95, 0x01, //   Report Count (1)
        0x75, 0x08, //   Report Size (8)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0x05, 0x09, //   Usage Page (Button)
        0x09, 0x01, //   Usage (Button 1)
        0x25, 0x01, //   Logical Maximum (1)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0x95, 0x07, //   Report Count (7)
        0x81, 0x03, //   Input (Const) — pad
        0x05, 0x0D, //   Usage Page (Digitizers)
        0x85, REPORT_ID_MAX_COUNT,
        0x09, 0x55, //   Usage (Contact Count Maximum)
        0x09, 0x59, //   Usage (Pad Type)
        0x75, 0x04, //   Report Size (4)
        0x95, 0x02, //   Report Count (2)
        0x25, 0x0F, //   Logical Maximum (15)
        0xB1, 0x02, //   Feature (Data,Var,Abs)
        0x06, 0x00, 0xFF, // Usage Page (Vendor Defined)
        0x85, REPORT_ID_PTPHQA,
        0x09, 0xC5, //   Usage (Device Certification Status)
        0x15, 0x00, //   Logical Minimum (0)
        0x26, 0xFF, 0x00, // Logical Maximum (255)
        0x75, 0x08, //   Report Size (8)
        0x96, 0x00, 0x01, // Report Count (256)
        0xB1, 0x02, //   Feature (Data,Var,Abs)
        0x05, 0x0D, //   Usage Page (Digitizers)
        0x85, REPORT_ID_LATENCY,
        0x09, 0x60, //   Usage (Latency Mode)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x01, //   Report Count (1)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x01, //   Logical Maximum (1)
        0xB1, 0x02, //   Feature (Data,Var,Abs)
        0x95, 0x07, //   Report Count (7)
        0xB1, 0x03, //   Feature (Const) — pad
        0xC0, // End Collection (Touch Pad)
        // Configuration TLC
        0x05, 0x0D, // Usage Page (Digitizers)
        0x09, 0x0E, // Usage (Configuration)
        0xA1, 0x01, // Collection (Application)
        0x85, REPORT_ID_INPUT_MODE,
        0x09, 0x22, //   Usage (Finger)
        0xA1, 0x02, //   Collection (Logical)
        0x09, 0x52, //     Usage (Input Mode)
        0x15, 0x00, //     Logical Minimum (0)
        0x25, 0x0A, //     Logical Maximum (10)
        0x75, 0x08, //     Report Size (8)
        0x95, 0x01, //     Report Count (1)
        0xB1, 0x02, //     Feature (Data,Var,Abs)
        0xC0, //         End Collection
        0x09, 0x22, //   Usage (Finger)
        0xA1, 0x00, //   Collection (Physical)
        0x85, REPORT_ID_FUNCTION_SWITCH,
        0x09, 0x57, //     Usage (Surface Switch)
        0x09, 0x58, //     Usage (Button Switch)
        0x75, 0x01, //     Report Size (1)
        0x95, 0x02, //     Report Count (2)
        0x25, 0x01, //     Logical Maximum (1)
        0xB1, 0x02, //     Feature (Data,Var,Abs)
        0x95, 0x06, //     Report Count (6)
        0xB1, 0x03, //     Feature (Const) — pad
        0xC0, //         End Collection
        0xC0, // End Collection (Configuration)
        // Mouse TLC: required by the PTP guide for hosts that never switch
        // input mode; we never send a mouse report.
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xA1, 0x01, // Collection (Application)
        0x85, REPORT_ID_MOUSE,
        0x09, 0x01, //   Usage (Pointer)
        0xA1, 0x00, //   Collection (Physical)
        0x05, 0x09, //     Usage Page (Button)
        0x19, 0x01, //     Usage Minimum (Button 1)
        0x29, 0x02, //     Usage Maximum (Button 2)
        0x25, 0x01, //     Logical Maximum (1)
        0x75, 0x01, //     Report Size (1)
        0x95, 0x02, //     Report Count (2)
        0x81, 0x02, //     Input (Data,Var,Abs)
        0x95, 0x06, //     Report Count (6)
        0x81, 0x03, //     Input (Const) — pad
        0x05, 0x01, //     Usage Page (Generic Desktop)
        0x09, 0x30, //     Usage (X)
        0x09, 0x31, //     Usage (Y)
        0x15, 0x81, //     Logical Minimum (-127)
        0x25, 0x7F, //     Logical Maximum (127)
        0x75, 0x08, //     Report Size (8)
        0x95, 0x02, //     Report Count (2)
        0x81, 0x06, //     Input (Data,Var,Rel)
        0xC0, //         End Collection
        0xC0, // End Collection (Mouse)
    ]);
    d
}

/// Pack a touch frame into the PTP input report.
pub fn pack_report(frame: &TouchFrame, scan_time_100us: u16) -> [u8; INPUT_REPORT_LEN] {
    let mut r = [0u8; INPUT_REPORT_LEN];
    r[0] = REPORT_ID_TOUCHPAD;
    for (i, c) in frame.contacts.iter().enumerate() {
        let at = 1 + i * CONTACT_LEN;
        // Confidence is always set: the tablet's own palm logic already
        // decides what reaches the host.
        r[at] = 1 | ((c.tip as u8) << 1);
        r[at + 1] = c.id;
        let x = c.x.max(0) as u16;
        let y = c.y.max(0) as u16;
        r[at + 2] = x as u8;
        r[at + 3] = (x >> 8) as u8;
        r[at + 4] = y as u8;
        r[at + 5] = (y >> 8) as u8;
    }
    let st = 1 + MAX_CONTACTS * CONTACT_LEN;
    r[st] = scan_time_100us as u8;
    r[st + 1] = (scan_time_100us >> 8) as u8;
    r[st + 2] = frame.count;
    r[st + 3] = 0; // button 1
    r
}

/// Feature reports for the PTP interface. Values written by the host are
/// stored so a later GET_REPORT reads them back, as the PTP guide requires.
pub struct Features {
    state: Mutex<FeatureState>,
}

#[derive(Debug, Clone, Copy)]
struct FeatureState {
    input_mode: u8,
    surface_switch: bool,
    button_switch: bool,
    latency_high: bool,
}

impl Features {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FeatureState {
                input_mode: 0,
                surface_switch: true,
                button_switch: true,
                latency_high: false,
            }),
        }
    }

    /// Whether the host has switched this interface into touchpad mode.
    #[allow(dead_code)]
    pub fn touchpad_mode(&self) -> bool {
        self.state.lock().unwrap().input_mode == INPUT_MODE_TOUCHPAD
    }
}

impl Default for Features {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureReports for Features {
    fn get(&self, report_id: u8) -> Option<Vec<u8>> {
        let s = *self.state.lock().unwrap();
        match report_id {
            REPORT_ID_MAX_COUNT => Some(vec![report_id, MAX_CONTACTS as u8]),
            REPORT_ID_PTPHQA => {
                let mut v = Vec::with_capacity(257);
                v.push(report_id);
                v.extend_from_slice(&CERTIFICATION_BLOB);
                Some(v)
            }
            REPORT_ID_INPUT_MODE => Some(vec![report_id, s.input_mode]),
            REPORT_ID_FUNCTION_SWITCH => Some(vec![
                report_id,
                s.surface_switch as u8 | ((s.button_switch as u8) << 1),
            ]),
            REPORT_ID_LATENCY => Some(vec![report_id, s.latency_high as u8]),
            _ => None,
        }
    }

    fn set(&self, report_id: u8, data: &[u8]) {
        // `data` starts with the report id.
        let Some(&value) = data.get(1) else { return };
        let mut s = self.state.lock().unwrap();
        match report_id {
            REPORT_ID_INPUT_MODE => {
                s.input_mode = value;
                log::info!(
                    "Host set touchpad input mode {} ({})",
                    value,
                    if value == INPUT_MODE_TOUCHPAD { "precision touchpad" } else { "mouse" }
                );
            }
            REPORT_ID_FUNCTION_SWITCH => {
                s.surface_switch = value & 1 != 0;
                s.button_switch = value & 2 != 0;
                log::debug!("Host set surface_switch={} button_switch={}", s.surface_switch, s.button_switch);
            }
            REPORT_ID_LATENCY => {
                s.latency_high = value & 1 != 0;
                log::debug!("Host set latency mode high={}", s.latency_high);
            }
            _ => {}
        }
    }
}

/// Microsoft's canonical PTP device certification blob. Optional on
/// Windows 10+, required for Windows 8.1-era certification checks.
pub const CERTIFICATION_BLOB: [u8; 256] = [
    0xfc, 0x28, 0xfe, 0x84, 0x40, 0xcb, 0x9a, 0x87, 0x0d, 0xbe, 0x57, 0x3c, 0xb6, 0x70, 0x09, 0x88,
    0x07, 0x97, 0x2d, 0x2b, 0xe3, 0x38, 0x34, 0xb6, 0x6c, 0xed, 0xb0, 0xf7, 0xe5, 0x9c, 0xf6, 0xc2,
    0x2e, 0x84, 0x1b, 0xe8, 0xb4, 0x51, 0x78, 0x43, 0x1f, 0x28, 0x4b, 0x7c, 0x2d, 0x53, 0xaf, 0xfc,
    0x47, 0x70, 0x1b, 0x59, 0x6f, 0x74, 0x43, 0xc4, 0xf3, 0x47, 0x18, 0x53, 0x1a, 0xa2, 0xa1, 0x71,
    0xc7, 0x95, 0x0e, 0x31, 0x55, 0x21, 0xd3, 0xb5, 0x1e, 0xe9, 0x0c, 0xba, 0xec, 0xb8, 0x89, 0x19,
    0x3e, 0xb3, 0xaf, 0x75, 0x81, 0x9d, 0x53, 0xb9, 0x41, 0x57, 0xf4, 0x6d, 0x39, 0x25, 0x29, 0x7c,
    0x87, 0xd9, 0xb4, 0x98, 0x45, 0x7d, 0xa7, 0x26, 0x9c, 0x65, 0x3b, 0x85, 0x68, 0x89, 0xd7, 0x3b,
    0xbd, 0xff, 0x14, 0x67, 0xf2, 0x2b, 0xf0, 0x2a, 0x41, 0x54, 0xf0, 0xfd, 0x2c, 0x66, 0x7c, 0xf8,
    0xc0, 0x8f, 0x33, 0x13, 0x03, 0xf1, 0xd3, 0xc1, 0x0b, 0x89, 0xd9, 0x1b, 0x62, 0xcd, 0x51, 0xb7,
    0x80, 0xb8, 0xaf, 0x3a, 0x10, 0xc1, 0x8a, 0x5b, 0xe8, 0x8a, 0x56, 0xf0, 0x8c, 0xaa, 0xfa, 0x35,
    0xe9, 0x42, 0xc4, 0xd8, 0x55, 0xc3, 0x38, 0xcc, 0x2b, 0x53, 0x5c, 0x69, 0x52, 0xd5, 0xc8, 0x73,
    0x02, 0x38, 0x7c, 0x73, 0xb6, 0x41, 0xe7, 0xff, 0x05, 0xd8, 0x2b, 0x79, 0x9a, 0xe2, 0x34, 0x60,
    0x8f, 0xa3, 0x32, 0x1f, 0x09, 0x78, 0x62, 0xbc, 0x80, 0xe3, 0x0f, 0xbd, 0x65, 0x20, 0x08, 0x13,
    0xc1, 0xe2, 0xee, 0x53, 0x2d, 0x86, 0x7e, 0xa7, 0x5a, 0xc5, 0xd3, 0x7d, 0x98, 0xbe, 0x31, 0x48,
    0x1f, 0xfb, 0xda, 0xaf, 0xa2, 0xa8, 0x6a, 0x89, 0xd6, 0xbf, 0xf2, 0xd3, 0x32, 0x2a, 0x9a, 0xe4,
    0xcf, 0x17, 0xb7, 0xb8, 0xf4, 0xe1, 0x33, 0x08, 0x24, 0x8b, 0xc4, 0x43, 0xa5, 0xe5, 0x24, 0xc2,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::Contact;

    fn geo() -> TouchGeometry {
        // rM2 in landscape: 1871 x 1403 units at 9 units/mm.
        TouchGeometry { x_max: 1871, y_max: 1403, resolution: 9 }
    }

    #[test]
    fn descriptor_declares_contact_count_max_and_physical_size() {
        let d = report_descriptor(&geo());
        // Contact Count Maximum feature under report id 2 with a 4-bit pair.
        assert!(d.windows(4).any(|w| w == [0x85, REPORT_ID_MAX_COUNT, 0x09, 0x55]));
        // 1871/9 = 207 mm, 1403/9 = 155 mm.
        assert!(d.windows(3).any(|w| w == [0x46, 207, 0]));
        assert!(d.windows(3).any(|w| w == [0x46, 155, 0]));
        // Five finger contacts (one Tip Switch usage each).
        assert_eq!(d.windows(2).filter(|w| *w == [0x09, 0x42]).count(), MAX_CONTACTS);
        // Certification blob of 256 bytes under report id 3.
        assert!(d.windows(3).any(|w| w == [0x96, 0x00, 0x01]));
    }

    #[test]
    fn packs_two_fingers() {
        let mut f = TouchFrame::default();
        f.contacts[0] = Contact { id: 0, tip: true, x: 0x0102, y: 0x0304 };
        f.contacts[2] = Contact { id: 2, tip: true, x: 10, y: 20 };
        f.count = 2;
        let r = pack_report(&f, 0xBEEF);
        assert_eq!(r.len(), INPUT_REPORT_LEN);
        assert_eq!(r[0], REPORT_ID_TOUCHPAD);
        assert_eq!(&r[1..7], &[0b11, 0, 0x02, 0x01, 0x04, 0x03]);
        assert_eq!(&r[7..13], &[0b01, 0, 0, 0, 0, 0]); // slot 1: confident, no tip
        assert_eq!(&r[13..19], &[0b11, 2, 10, 0, 20, 0]);
        assert_eq!(&r[31..33], &[0xEF, 0xBE]);
        assert_eq!(r[33], 2);
        assert_eq!(r[34], 0);
    }

    #[test]
    fn feature_roundtrip_input_mode() {
        let f = Features::new();
        assert_eq!(f.get(REPORT_ID_INPUT_MODE), Some(vec![REPORT_ID_INPUT_MODE, 0]));
        f.set(REPORT_ID_INPUT_MODE, &[REPORT_ID_INPUT_MODE, INPUT_MODE_TOUCHPAD]);
        assert!(f.touchpad_mode());
        assert_eq!(f.get(REPORT_ID_MAX_COUNT), Some(vec![REPORT_ID_MAX_COUNT, 5]));
        assert_eq!(f.get(REPORT_ID_PTPHQA).unwrap().len(), 257);
        assert_eq!(f.get(REPORT_ID_MOUSE), None);
    }
}
