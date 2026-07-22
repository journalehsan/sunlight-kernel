//! Gregorian weekday conversion used by the shell's date presentation.
//!
//! `weekday_sun0` is for the Sunday-first month grid; `weekday_iso` is for
//! locale text, whose ABI is Monday=1 through Sunday=7.  Presentation order
//! must not change the mathematical weekday.

pub fn weekday_sun0(year: u16, month: u8, day: u8) -> usize {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    let m = month as i32;
    if m < 3 {
        y -= 1;
    }
    ((y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + day as i32) % 7) as usize
}

pub fn weekday_iso(year: u16, month: u8, day: u8) -> u8 {
    match weekday_sun0(year, month, day) {
        0 => 7,
        weekday => weekday as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates_keep_their_mathematical_weekday() {
        // 1970-01-01 was Thursday; 2026-07-01 is Wednesday.
        assert_eq!(weekday_sun0(1970, 1, 1), 4);
        assert_eq!(weekday_iso(1970, 1, 1), 4);
        assert_eq!(weekday_sun0(2026, 7, 1), 3);
        assert_eq!(weekday_iso(2026, 7, 1), 3);
        // Sunday remains ISO day 7, not the shell's Sunday-first index 0.
        assert_eq!(weekday_iso(2026, 7, 5), 7);
    }
}
