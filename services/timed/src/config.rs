//! Timezone configuration parsing
//!
//! Safely parses timezone offset strings (float or integer) and DST flags
//! from configuration files.

/// Parse timezone offset from a string (may be float or integer)
/// Returns offset in seconds
///
/// # Examples
/// - "+4.5" → 16200 seconds (4.5 hours)
/// - "-5" → -18000 seconds (5 hours behind)
/// - "8.0" → 28800 seconds (8 hours)
pub fn parse_offset_string(s: &str) -> Result<i32, &'static str> {
    let trimmed = s.trim();

    if trimmed.is_empty() {
        return Err("Empty offset string");
    }

    // Try parsing as integer first
    if let Ok(int_val) = parse_i32(trimmed) {
        return Ok(int_val * 3600); // Convert hours to seconds
    }

    // Try parsing as float
    if let Ok(float_val) = parse_f64(trimmed) {
        let seconds = (float_val * 3600.0) as i32;
        return Ok(seconds);
    }

    Err("Invalid offset format")
}

/// Parse DST flag from a string (1 = active, 0 or missing = inactive)
pub fn parse_dst_flag(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed == "1"
}

/// Safe i32 parsing without using parse()
/// Handles signs and basic validation
fn parse_i32(s: &str) -> Result<i32, &'static str> {
    let mut chars = s.chars();
    let mut is_negative = false;
    let mut result: i32 = 0;

    if s.is_empty() {
        return Err("Empty string");
    }

    let first_char = chars.next().unwrap();
    if first_char == '+' {
        is_negative = false;
    } else if first_char == '-' {
        is_negative = true;
    } else if first_char.is_ascii_digit() {
        result = (first_char as i32) - (b'0' as i32);
    } else {
        return Err("Invalid number format");
    }

    for c in chars {
        if !c.is_ascii_digit() {
            // For integers, any non-digit is invalid
            return Err("Non-digit character in integer");
        }
        result = result.saturating_mul(10);
        result = result.saturating_add((c as i32) - (b'0' as i32));
    }

    Ok(if is_negative { -result } else { result })
}

/// Safe f64 parsing without using parse()
/// Handles signs, decimal points, and basic validation
fn parse_f64(s: &str) -> Result<f64, &'static str> {
    let mut chars = s.chars().peekable();
    let mut is_negative = false;
    let mut integer_part: i32 = 0;
    let mut fractional_part: i32 = 0;
    let mut decimal_places: i32 = 0;
    let mut has_dot = false;
    let mut has_digit = false;

    if s.is_empty() {
        return Err("Empty string");
    }

    // Check sign
    if let Some(&c) = chars.peek() {
        if c == '+' {
            is_negative = false;
            chars.next();
        } else if c == '-' {
            is_negative = true;
            chars.next();
        }
    }

    // Parse integer part and fractional part
    while let Some(c) = chars.next() {
        if c == '.' {
            if has_dot {
                return Err("Multiple decimal points");
            }
            has_dot = true;
        } else if c.is_ascii_digit() {
            has_digit = true;
            let digit = (c as i32) - (b'0' as i32);

            if has_dot {
                fractional_part = fractional_part.saturating_mul(10);
                fractional_part = fractional_part.saturating_add(digit);
                decimal_places += 1;
            } else {
                integer_part = integer_part.saturating_mul(10);
                integer_part = integer_part.saturating_add(digit);
            }
        } else {
            return Err("Invalid character in float");
        }
    }

    if !has_digit {
        return Err("No digits found");
    }

    // Reconstruct float value
    let mut result = integer_part as f64;
    if decimal_places > 0 {
        // Manual power calculation: 10^n
        let mut divisor = 1.0;
        for _ in 0..decimal_places {
            divisor *= 10.0;
        }
        result += (fractional_part as f64) / divisor;
    }

    Ok(if is_negative { -result } else { result })
}

/// Validate timezone offset is in reasonable range
pub fn validate_offset(offset_secs: i32) -> bool {
    // Allow range from -12 hours to +14 hours
    let min = -12 * 3600;
    let max = 14 * 3600;
    offset_secs >= min && offset_secs <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_offset_positive_float() {
        assert_eq!(parse_offset_string("+4.5"), Ok(16200));
    }

    #[test]
    fn test_parse_offset_negative_int() {
        assert_eq!(parse_offset_string("-5"), Ok(-18000));
    }

    #[test]
    fn test_parse_offset_positive_int() {
        assert_eq!(parse_offset_string("8"), Ok(28800));
    }

    #[test]
    fn test_parse_offset_float_no_sign() {
        assert_eq!(parse_offset_string("5.5"), Ok(19800));
    }

    #[test]
    fn test_parse_offset_zero() {
        assert_eq!(parse_offset_string("0"), Ok(0));
    }

    #[test]
    fn test_parse_dst_flag_true() {
        assert!(parse_dst_flag("1"));
    }

    #[test]
    fn test_parse_dst_flag_false() {
        assert!(!parse_dst_flag("0"));
        assert!(!parse_dst_flag(""));
        assert!(!parse_dst_flag("anything"));
    }

    #[test]
    fn test_validate_offset_valid() {
        assert!(validate_offset(0));
        assert!(validate_offset(16200)); // +4.5h
        assert!(validate_offset(-18000)); // -5h
        assert!(validate_offset(50400)); // +14h
        assert!(validate_offset(-43200)); // -12h
    }

    #[test]
    fn test_validate_offset_invalid() {
        assert!(!validate_offset(60000)); // > +14h
        assert!(!validate_offset(-60000)); // < -12h
    }
}
