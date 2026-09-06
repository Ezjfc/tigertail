//! HID pen digitizer: report descriptor and input-report packing.
//!
//! One application collection (Digitizers / Pen) with a single unnumbered
//! 9-byte input report:
//!
//! ```text
//! byte 0   bit0 tip switch, bit1 barrel switch, bit2 invert (eraser end),
//!          bit3 eraser (erasing contact), bit4 in range, bits 5-7 pad
//! byte 1-2 X (little endian, 0..x_max)
//! byte 3-4 Y (little endian, 0..y_max)
//! byte 5-6 tip pressure (0..pressure_max)
//! byte 7   X tilt in degrees (signed)
//! byte 8   Y tilt in degrees (signed)
//! ```
//!
//! Every field is standard HID digitizer usage, so Windows/Linux/macOS bind
//! their in-box pen drivers with no host-side software.

use crate::pen::{PenFrame, PenGeometry};

pub const REPORT_LEN: usize = 9;

/// Build the report descriptor for the given (transformed) pen geometry.
pub fn report_descriptor(geo: &PenGeometry) -> Vec<u8> {
    let x_max = geo.x_max as u16;
    let y_max = geo.y_max as u16;
    let pressure_max = geo.pressure_max as u16;
    // Device tilt units are centidegrees on every supported model; HID wants
    // plain degrees.
    let tilt_deg = (geo.tilt_range / 100).clamp(1, 90) as u8;

    let mut d = vec![
        0x05, 0x0D, // Usage Page (Digitizers)
        0x09, 0x02, // Usage (Pen)
        0xA1, 0x01, // Collection (Application)
        0x09, 0x20, //   Usage (Stylus)
        0xA1, 0x00, //   Collection (Physical)
        0x09, 0x42, //     Usage (Tip Switch)
        0x09, 0x44, //     Usage (Barrel Switch)
        0x09, 0x3C, //     Usage (Invert)
        0x09, 0x45, //     Usage (Eraser)
        0x09, 0x32, //     Usage (In Range)
        0x15, 0x00, //     Logical Minimum (0)
        0x25, 0x01, //     Logical Maximum (1)
        0x75, 0x01, //     Report Size (1)
        0x95, 0x05, //     Report Count (5)
        0x81, 0x02, //     Input (Data,Var,Abs)
        0x75, 0x03, //     Report Size (3)
        0x95, 0x01, //     Report Count (1)
        0x81, 0x03, //     Input (Const) — pad to a byte
        0x05, 0x01, //     Usage Page (Generic Desktop)
        0x09, 0x30, //     Usage (X)
        0x15, 0x00, //     Logical Minimum (0)
    ];
    d.extend_from_slice(&[0x26, x_max as u8, (x_max >> 8) as u8]); // Logical Maximum (x_max)
    d.extend_from_slice(&[
        0x75, 0x10, //     Report Size (16)
        0x95, 0x01, //     Report Count (1)
        0x81, 0x02, //     Input (Data,Var,Abs)
        0x09, 0x31, //     Usage (Y)
    ]);
    d.extend_from_slice(&[0x26, y_max as u8, (y_max >> 8) as u8]); // Logical Maximum (y_max)
    d.extend_from_slice(&[
        0x75, 0x10, //     Report Size (16)
        0x95, 0x01, //     Report Count (1)
        0x81, 0x02, //     Input (Data,Var,Abs)
        0x05, 0x0D, //     Usage Page (Digitizers)
        0x09, 0x30, //     Usage (Tip Pressure)
    ]);
    d.extend_from_slice(&[0x26, pressure_max as u8, (pressure_max >> 8) as u8]);
    d.extend_from_slice(&[
        0x75, 0x10, //     Report Size (16)
        0x95, 0x01, //     Report Count (1)
        0x81, 0x02, //     Input (Data,Var,Abs)
        0x09, 0x3D, //     Usage (X Tilt)
    ]);
    d.extend_from_slice(&[
        0x15,
        (-(tilt_deg as i8)) as u8, // Logical Minimum (-tilt_deg)
        0x25,
        tilt_deg, //           Logical Maximum (tilt_deg)
        0x75,
        0x08, //               Report Size (8)
        0x95,
        0x01, //               Report Count (1)
        0x81,
        0x02, //               Input (Data,Var,Abs)
        0x09,
        0x3E, //               Usage (Y Tilt)
        0x81,
        0x02, //               Input (Data,Var,Abs)
    ]);
    d.extend_from_slice(&[
        0xC0, //   End Collection (Physical)
        0xC0, // End Collection (Application)
    ]);
    d
}

/// Pack a transformed pen frame into the 9-byte input report.
pub fn pack_report(frame: &PenFrame, geo: &PenGeometry) -> [u8; REPORT_LEN] {
    let mut buttons = 0u8;
    // With the eraser end down, contact reports as "eraser", not "tip".
    if frame.tip && !frame.eraser {
        buttons |= 1 << 0;
    }
    if frame.barrel {
        buttons |= 1 << 1;
    }
    if frame.eraser {
        buttons |= 1 << 2; // invert: eraser end is toward the surface
    }
    if frame.tip && frame.eraser {
        buttons |= 1 << 3; // erasing contact
    }
    if frame.in_range {
        buttons |= 1 << 4;
    }

    let x = frame.x.clamp(0, geo.x_max) as u16;
    let y = frame.y.clamp(0, geo.y_max) as u16;
    let pressure = frame.pressure.clamp(0, geo.pressure_max) as u16;
    let tilt_deg = |t: i32| ((t / 100).clamp(-90, 90) as i8) as u8;

    [
        buttons,
        x as u8,
        (x >> 8) as u8,
        y as u8,
        (y >> 8) as u8,
        pressure as u8,
        (pressure >> 8) as u8,
        tilt_deg(frame.tilt_x),
        tilt_deg(frame.tilt_y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> PenGeometry {
        PenGeometry {
            x_max: 20967,
            y_max: 15725,
            pressure_max: 4095,
            distance_max: 255,
            tilt_range: 6400,
        }
    }

    #[test]
    fn descriptor_embeds_logical_maxima() {
        let d = report_descriptor(&geo());
        // 20967 = 0x51E7, 15725 = 0x3D6D, 4095 = 0x0FFF little-endian after 0x26.
        assert!(d.windows(3).any(|w| w == [0x26, 0xE7, 0x51]));
        assert!(d.windows(3).any(|w| w == [0x26, 0x6D, 0x3D]));
        assert!(d.windows(3).any(|w| w == [0x26, 0xFF, 0x0F]));
        // rM2 tilt range 6400 centidegrees -> ±64°.
        assert!(d.windows(2).any(|w| w == [0x25, 64]));
    }

    #[test]
    fn pack_pen_contact() {
        let f = PenFrame {
            x: 0x1234,
            y: 0x0567,
            pressure: 1000,
            tilt_x: 4500,  // 45°
            tilt_y: -4500, // -45°
            in_range: true,
            tip: true,
            ..Default::default()
        };
        let r = pack_report(&f, &geo());
        assert_eq!(r[0], 0b0001_0001); // tip + in-range
        assert_eq!([r[1], r[2]], [0x34, 0x12]);
        assert_eq!([r[3], r[4]], [0x67, 0x05]);
        assert_eq!([r[5], r[6]], [0xE8, 0x03]);
        assert_eq!(r[7] as i8, 45);
        assert_eq!(r[8] as i8, -45);
    }

    #[test]
    fn pack_eraser_contact_reports_invert_and_eraser_not_tip() {
        let f = PenFrame {
            in_range: true,
            tip: true,
            eraser: true,
            ..Default::default()
        };
        let r = pack_report(&f, &geo());
        assert_eq!(r[0], 0b0001_1100); // invert + eraser + in-range, no tip
    }
}
