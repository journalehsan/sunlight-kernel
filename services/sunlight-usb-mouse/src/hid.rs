//! Allocation-free HID input-field decoding.
//!
//! The current xHCI driver deliberately selects HID boot-mouse protocol.  The
//! boot layout below is therefore fixed by that protocol, not guessed from a
//! report-protocol packet.  The bit decoder remains layout-driven so signed and
//! non-byte-aligned fields can be tested without adding work to the hot path.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HidField {
    pub bit_offset: u16,
    pub bit_width: u8,
    pub logical_minimum: i32,
    pub logical_maximum: i32,
}

impl HidField {
    const fn is_signed(self) -> bool {
        self.logical_minimum < 0
    }

    const fn end_bit(self) -> Option<usize> {
        (self.bit_offset as usize).checked_add(self.bit_width as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseReportLayout {
    /// Report IDs are absent in boot protocol. This remains explicit so the
    /// decoder cannot silently shift fields for a report-protocol layout.
    pub report_id: Option<u8>,
    pub buttons: HidField,
    pub x: HidField,
    pub y: HidField,
    /// A fourth boot-compatible byte is treated as an optional wheel field.
    pub wheel: Option<HidField>,
}

/// HID boot-mouse packet: buttons at bit 0, signed relative X at bit 8, and
/// signed relative Y at bit 16. X/Y are eight bits with the conventional
/// descriptor range -127..=127. A compatible wheel byte may follow at bit 24.
pub const BOOT_MOUSE_LAYOUT: MouseReportLayout = MouseReportLayout {
    report_id: None,
    buttons: HidField {
        bit_offset: 0,
        bit_width: 8,
        logical_minimum: 0,
        logical_maximum: 255,
    },
    x: HidField {
        bit_offset: 8,
        bit_width: 8,
        logical_minimum: -127,
        logical_maximum: 127,
    },
    y: HidField {
        bit_offset: 16,
        bit_width: 8,
        logical_minimum: -127,
        logical_maximum: 127,
    },
    wheel: Some(HidField {
        bit_offset: 24,
        bit_width: 8,
        logical_minimum: -127,
        logical_maximum: 127,
    }),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedMouseReport {
    pub report_id: Option<u8>,
    pub buttons: u8,
    pub raw_x: u64,
    pub raw_y: u64,
    pub dx: i16,
    pub dy: i16,
    pub wheel: Option<i16>,
}

/// Extract an HID field in the specification's little-endian, least-significant
/// bit first order. Invalid widths and short reports are rejected before any
/// indexing or shifting occurs.
pub fn extract_bits(report: &[u8], bit_offset: usize, bit_width: u8) -> Option<u64> {
    if bit_width == 0 || bit_width > 64 {
        return None;
    }
    let end_bit = bit_offset.checked_add(bit_width as usize)?;
    if end_bit > report.len().checked_mul(8)? {
        return None;
    }

    let mut value = 0u64;
    let mut output_bit = 0u32;
    while output_bit < bit_width as u32 {
        let input_bit = bit_offset + output_bit as usize;
        let bit = (report[input_bit / 8] >> (input_bit % 8)) & 1;
        value |= u64::from(bit) << output_bit;
        output_bit += 1;
    }
    Some(value)
}

/// Sign-extend an N-bit two's-complement field without an N-bit shift in the
/// storage type. The i128 intermediate safely covers the full u64 input width.
pub fn sign_extend(raw: u64, bit_width: u8) -> Option<i64> {
    if bit_width == 0 || bit_width > 64 {
        return None;
    }

    let mask = if bit_width == 64 {
        u64::MAX
    } else {
        (1u64 << bit_width) - 1
    };
    let value = raw & mask;
    let sign_bit = 1u64 << (bit_width - 1);
    if value & sign_bit == 0 {
        i64::try_from(value).ok()
    } else {
        i64::try_from((value as i128) - (1i128 << bit_width)).ok()
    }
}

fn decode_field(report: &[u8], field: HidField) -> Option<(u64, i64)> {
    let raw = extract_bits(report, field.bit_offset as usize, field.bit_width)?;
    let decoded = if field.is_signed() {
        sign_extend(raw, field.bit_width)?
    } else {
        i64::try_from(raw).ok()?
    };
    Some((raw, decoded))
}

pub fn decode_mouse_report(
    report: &[u8],
    layout: &MouseReportLayout,
) -> Option<DecodedMouseReport> {
    let (report_id, payload) = match layout.report_id {
        Some(expected) if report.first().copied() == Some(expected) => {
            (Some(expected), report.get(1..)?)
        }
        Some(_) => return None,
        None => (None, report),
    };

    // Buttons and both axes are mandatory. The wheel is deliberately optional
    // for the three-byte boot report accepted by the current driver.
    let required_bits = layout
        .buttons
        .end_bit()?
        .max(layout.x.end_bit()?)
        .max(layout.y.end_bit()?);
    if required_bits > payload.len().checked_mul(8)? {
        return None;
    }

    let (_, buttons) = decode_field(payload, layout.buttons)?;
    let (raw_x, dx) = decode_field(payload, layout.x)?;
    let (raw_y, dy) = decode_field(payload, layout.y)?;
    let wheel = layout
        .wheel
        .and_then(|field| decode_field(payload, field).map(|(_, value)| value))
        .and_then(|value| i16::try_from(value).ok());

    Some(DecodedMouseReport {
        report_id,
        buttons: u8::try_from(buttons).ok()?,
        raw_x,
        raw_y,
        dx: i16::try_from(dx).ok()?,
        dy: i16::try_from(dy).ok()?,
        wheel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(report: &[u8]) -> DecodedMouseReport {
        decode_mouse_report(report, &BOOT_MOUSE_LAYOUT).expect("valid boot report")
    }

    #[test]
    fn decodes_positive_and_negative_eight_bit_axes() {
        let positive = boot(&[0, 12, 34]);
        assert_eq!((positive.dx, positive.dy), (12, 34));

        let negative = boot(&[0, 0xf4, 0xde]);
        assert_eq!((negative.dx, negative.dy), (-12, -34));
    }

    #[test]
    fn decodes_signed_eight_bit_boundaries() {
        for (raw, expected) in [(0x00, 0), (0x01, 1), (0xff, -1), (0x7f, 127), (0x80, -128)] {
            assert_eq!(sign_extend(raw, 8), Some(expected));
        }
        assert_eq!(boot(&[0, 0, 0xff]).dy, -1);
        assert_eq!(boot(&[0, 0, 0x80]).dy, -128);
    }

    #[test]
    fn decodes_non_byte_aligned_signed_fields() {
        let layout = MouseReportLayout {
            report_id: None,
            buttons: HidField {
                bit_offset: 0,
                bit_width: 3,
                logical_minimum: 0,
                logical_maximum: 7,
            },
            x: HidField {
                bit_offset: 3,
                bit_width: 5,
                logical_minimum: -16,
                logical_maximum: 15,
            },
            y: HidField {
                bit_offset: 8,
                bit_width: 5,
                logical_minimum: -16,
                logical_maximum: 15,
            },
            wheel: None,
        };
        // buttons=5, X=0b11111 (-1), Y=0b10000 (-16)
        let decoded = decode_mouse_report(&[0xfd, 0x10], &layout).unwrap();
        assert_eq!(decoded.buttons, 5);
        assert_eq!((decoded.raw_x, decoded.raw_y), (0x1f, 0x10));
        assert_eq!((decoded.dx, decoded.dy), (-1, -16));
    }

    #[test]
    fn honors_report_id_without_shifting_axes() {
        let layout = MouseReportLayout {
            report_id: Some(7),
            ..BOOT_MOUSE_LAYOUT
        };
        let decoded = decode_mouse_report(&[7, 1, 0xfe, 3, 4], &layout).unwrap();
        assert_eq!(decoded.report_id, Some(7));
        assert_eq!(
            (decoded.buttons, decoded.dx, decoded.dy, decoded.wheel),
            (1, -2, 3, Some(4))
        );
        assert!(decode_mouse_report(&[6, 1, 0xfe, 3], &layout).is_none());
    }

    #[test]
    fn wheel_after_axes_does_not_change_y() {
        let without_wheel = boot(&[0, 4, 0xfb]);
        let with_wheel = boot(&[0, 4, 0xfb, 0xff]);
        assert_eq!((without_wheel.dx, without_wheel.dy), (4, -5));
        assert_eq!((with_wheel.dx, with_wheel.dy), (4, -5));
        assert_eq!(without_wheel.wheel, None);
        assert_eq!(with_wheel.wheel, Some(-1));
    }

    #[test]
    fn rejects_reports_shorter_than_the_axes_require() {
        assert!(decode_mouse_report(&[], &BOOT_MOUSE_LAYOUT).is_none());
        assert!(decode_mouse_report(&[0], &BOOT_MOUSE_LAYOUT).is_none());
        assert!(decode_mouse_report(&[0, 1], &BOOT_MOUSE_LAYOUT).is_none());
    }

    #[test]
    fn synthetic_boot_report_replay_preserves_axes_and_clicks() {
        let cases = [
            (&[0, 0, 0xff][..], (0, -1, 0)),     // up
            (&[0, 0, 1][..], (0, 1, 0)),         // down
            (&[0, 0xff, 0][..], (-1, 0, 0)),     // left
            (&[0, 1, 0][..], (1, 0, 0)),         // right
            (&[0, 0xff, 0xff][..], (-1, -1, 0)), // up-left
            (&[0, 1, 1][..], (1, 1, 0)),         // down-right
            (&[1, 0, 0][..], (0, 0, 1)),         // click, no motion
        ];

        for (report, expected) in cases {
            let decoded = boot(report);
            assert_eq!((decoded.dx, decoded.dy, decoded.buttons & 7), expected);
        }
    }
}
