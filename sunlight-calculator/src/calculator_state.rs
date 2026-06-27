const DISPLAY_BUF_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    Digit(u8),
    Decimal,
    Add,
    Subtract,
    Multiply,
    Divide,
    Equals,
    Clear,
    MemoryClear,
    MemoryRecall,
    MemoryAdd,
    MemorySubtract,
    Sqrt,
    Percent,
    Reciprocal,
    Negate,
}

pub struct CalculatorState {
    pub display_buf: [u8; DISPLAY_BUF_SIZE],
    pub display_len: usize,
    pub error: bool,

    accumulator: Option<f64>,
    pending_op: Option<Operator>,
    last_op: Option<(Operator, f64)>,
    memory: f64,

    entering_new_operand: bool,
    just_evaluated: bool,
    has_decimal_point: bool,
}

impl CalculatorState {
    pub fn new() -> Self {
        let mut state = Self {
            display_buf: [0; DISPLAY_BUF_SIZE],
            display_len: 1,
            error: false,
            accumulator: None,
            pending_op: None,
            last_op: None,
            memory: 0.0,
            entering_new_operand: false,
            just_evaluated: false,
            has_decimal_point: false,
        };
        state.display_buf[0] = b'0';
        state
    }

    pub fn display_str(&self) -> &str {
        if self.error {
            return "Error";
        }
        core::str::from_utf8(&self.display_buf[..self.display_len]).unwrap_or("Error")
    }

    pub fn handle_action(&mut self, action: ButtonAction) {
        match action {
            ButtonAction::Digit(d) => self.digit(d),
            ButtonAction::Decimal => self.decimal(),
            ButtonAction::Add => self.handle_operator(Operator::Add),
            ButtonAction::Subtract => self.handle_operator(Operator::Subtract),
            ButtonAction::Multiply => self.handle_operator(Operator::Multiply),
            ButtonAction::Divide => self.handle_operator(Operator::Divide),
            ButtonAction::Equals => self.handle_equals(),
            ButtonAction::Clear => self.clear(),
            ButtonAction::MemoryClear => self.memory = 0.0,
            ButtonAction::MemoryRecall => self.memory_recall(),
            ButtonAction::MemoryAdd => self.memory_add(),
            ButtonAction::MemorySubtract => self.memory_subtract(),
            ButtonAction::Sqrt => self.handle_unary(|x| if x < 0.0 { f64::NAN } else { fsqrt(x) }),
            ButtonAction::Percent => self.handle_unary(|x| x / 100.0),
            ButtonAction::Reciprocal => self.handle_unary(|x| 1.0 / x),
            ButtonAction::Negate => self.negate(),
        }
    }

    fn digit(&mut self, d: u8) {
        if self.error {
            self.clear();
        }

        if self.just_evaluated {
            self.accumulator = None;
            self.last_op = None;
            self.pending_op = None;
            self.just_evaluated = false;
            self.input_reset();
        } else if self.entering_new_operand {
            self.input_reset();
        }

        if self.display_len >= DISPLAY_BUF_SIZE - 1 {
            return;
        }

        if self.display_len == 1 && self.display_buf[0] == b'0' && !self.has_decimal_point {
            self.display_buf[0] = b'0' + d;
            self.display_len = 1;
        } else {
            let idx = self.display_len;
            self.display_buf[idx] = b'0' + d;
            self.display_len += 1;
        }
    }

    fn decimal(&mut self) {
        if self.error {
            self.clear();
        }

        if self.just_evaluated {
            self.accumulator = None;
            self.last_op = None;
            self.pending_op = None;
            self.just_evaluated = false;
            self.input_reset();
        } else if self.entering_new_operand {
            self.input_reset();
        }

        if self.has_decimal_point {
            return;
        }
        if self.display_len >= DISPLAY_BUF_SIZE - 1 {
            return;
        }

        if self.display_len == 0 {
            self.display_buf[0] = b'0';
            self.display_len = 1;
        }

        self.display_buf[self.display_len] = b'.';
        self.display_len += 1;
        self.has_decimal_point = true;
    }

    fn negate(&mut self) {
        if self.error {
            return;
        }
        let len = self.display_len;
        if len == 0 || (len == 1 && self.display_buf[0] == b'0') {
            return;
        }
        if self.display_buf[0] == b'-' {
            for i in 1..len {
                self.display_buf[i - 1] = self.display_buf[i];
            }
            self.display_len -= 1;
        } else if len < DISPLAY_BUF_SIZE - 1 {
            for i in (0..len).rev() {
                self.display_buf[i + 1] = self.display_buf[i];
            }
            self.display_buf[0] = b'-';
            self.display_len += 1;
        }
    }

    fn handle_operator(&mut self, op: Operator) {
        if self.error {
            return;
        }

        if self.just_evaluated {
            self.just_evaluated = false;
            if self.entering_new_operand {
                self.pending_op = Some(op);
                self.last_op = None;
                return;
            }
        }

        if self.entering_new_operand {
            self.pending_op = Some(op);
            self.last_op = None;
            return;
        }

        if let Some(pending) = self.pending_op.take() {
            if let Some(acc) = self.accumulator {
                let rhs = self.current_value();
                if let Some(result) = Self::try_eval(acc, pending, rhs) {
                    self.set_display_f64(result);
                    self.accumulator = Some(result);
                } else {
                    self.set_error();
                    return;
                }
            }
        } else {
            self.accumulator = Some(self.current_value());
        }

        self.pending_op = Some(op);
        self.entering_new_operand = true;
        self.just_evaluated = false;
        self.last_op = None;
    }

    fn handle_equals(&mut self) {
        if self.error {
            return;
        }

        if let Some(op) = self.pending_op.take() {
            if let Some(acc) = self.accumulator {
                let rhs = self.current_value();
                if let Some(result) = Self::try_eval(acc, op, rhs) {
                    self.set_display_f64(result);
                    self.last_op = Some((op, rhs));
                    self.accumulator = Some(result);
                    self.entering_new_operand = true;
                    self.just_evaluated = true;
                } else {
                    self.set_error();
                }
            }
        } else if let Some((op, rhs)) = self.last_op {
            if let Some(acc) = self.accumulator {
                if let Some(result) = Self::try_eval(acc, op, rhs) {
                    self.set_display_f64(result);
                    self.accumulator = Some(result);
                    self.entering_new_operand = true;
                    self.just_evaluated = true;
                } else {
                    self.set_error();
                }
            }
        }
    }

    fn handle_unary<F>(&mut self, f: F)
    where
        F: Fn(f64) -> f64,
    {
        if self.error {
            return;
        }
        let val = self.current_value();
        if val_is_bad(val) {
            self.set_error();
            return;
        }
        let result = f(val);
        if val_is_bad(result) {
            self.set_error();
            return;
        }
        self.set_display_f64(result);
        self.entering_new_operand = true;
        self.just_evaluated = true;
    }

    fn memory_recall(&mut self) {
        if self.error {
            self.clear();
        }
        self.set_display_f64(self.memory);
        self.entering_new_operand = true;
        self.just_evaluated = false;
    }

    fn memory_add(&mut self) {
        if self.error {
            return;
        }
        self.memory += self.current_value();
    }

    fn memory_subtract(&mut self) {
        if self.error {
            return;
        }
        self.memory -= self.current_value();
    }

    fn clear(&mut self) {
        self.accumulator = None;
        self.pending_op = None;
        self.last_op = None;
        self.error = false;
        self.input_reset();
        self.display_buf[0] = b'0';
        self.display_len = 1;
    }

    fn input_reset(&mut self) {
        self.entering_new_operand = false;
        self.just_evaluated = false;
        self.has_decimal_point = false;
        self.display_len = 0;
    }

    fn current_value(&self) -> f64 {
        if self.display_len == 0 {
            return 0.0;
        }
        let s = match core::str::from_utf8(&self.display_buf[..self.display_len]) {
            Ok(s) => s,
            Err(_) => return 0.0,
        };
        s.parse::<f64>().unwrap_or(0.0)
    }

    fn set_display_f64(&mut self, value: f64) {
        fmt_f64(value, &mut self.display_buf, &mut self.display_len);
        self.rebuild_internals();
    }

    fn rebuild_internals(&mut self) {
        let s = match core::str::from_utf8(&self.display_buf[..self.display_len]) {
            Ok(s) => s,
            Err(_) => return,
        };
        if s == "Error" || s == "error" {
            self.error = true;
        } else {
            self.has_decimal_point = s.contains('.');
            self.error = false;
        }
    }

    fn set_error(&mut self) {
        self.error = true;
        let err = b"Error";
        self.display_len = err.len().min(DISPLAY_BUF_SIZE);
        self.display_buf[..self.display_len].copy_from_slice(&err[..self.display_len]);
    }

    fn try_eval(acc: f64, op: Operator, rhs: f64) -> Option<f64> {
        match op {
            Operator::Add => Some(acc + rhs),
            Operator::Subtract => Some(acc - rhs),
            Operator::Multiply => Some(acc * rhs),
            Operator::Divide => {
                if rhs == 0.0 {
                    None
                } else {
                    Some(acc / rhs)
                }
            }
        }
    }

    pub fn memory_value(&self) -> f64 {
        self.memory
    }
}

// ── Float helpers (no dependency on unavailable f64 intrinsics) ────────────────

fn val_is_bad(x: f64) -> bool {
    x != x || x == f64::INFINITY || x == f64::NEG_INFINITY
}

fn ftrunc(x: f64) -> f64 {
    if x >= 0.0 {
        (x as u64) as f64
    } else if x > -1.0 {
        -0.0f64
    } else {
        -(((-x) as u64) as f64)
    }
}

fn ffract(x: f64) -> f64 {
    x - ftrunc(x)
}

fn fsqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x;
    for _ in 0..25 {
        guess = (guess + x / guess) * 0.5;
    }
    guess
}

fn fmt_f64(value: f64, buf: &mut [u8; DISPLAY_BUF_SIZE], len: &mut usize) {
    if val_is_bad(value) {
        let err = b"Error";
        *len = err.len().min(buf.len());
        buf[..*len].copy_from_slice(&err[..*len]);
        return;
    }

    let abs_val = if value < 0.0 { -value } else { value };
    let is_neg = value < 0.0;

    if abs_val == 0.0 {
        buf[0] = b'0';
        *len = 1;
        return;
    }

    if abs_val >= 1e15 {
        buf[0] = b'0';
        *len = 1;
        return;
    }

    let mut tmp = [0u8; 32];

    let int_part = ftrunc(abs_val) as u64;
    let frac_part = ffract(abs_val);

    let mut tmp_len = write_u64(int_part, &mut tmp);

    if frac_part != 0.0 {
        if tmp_len < tmp.len() {
            tmp[tmp_len] = b'.';
            tmp_len += 1;
        }
        let mut frac = frac_part;
        let frac_start = tmp_len;
        for _ in 0..10 {
            if frac == 0.0 {
                break;
            }
            if tmp_len >= tmp.len() - 1 {
                break;
            }
            frac *= 10.0;
            let digit = ftrunc(frac) as u8;
            tmp[tmp_len] = b'0' + digit;
            tmp_len += 1;
            frac -= digit as f64;
        }

        while tmp_len > frac_start + 1 && tmp[tmp_len - 1] == b'0' {
            tmp_len -= 1;
        }
    }

    let mut out_pos = 0;
    if is_neg && out_pos < buf.len() {
        buf[out_pos] = b'-';
        out_pos += 1;
    }

    let copy_len = (tmp_len).min(buf.len().saturating_sub(out_pos));
    buf[out_pos..out_pos + copy_len].copy_from_slice(&tmp[..copy_len]);
    *len = out_pos + copy_len;
}

fn write_u64(mut n: u64, buf: &mut [u8]) -> usize {
    if n == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut tmp = [0u8; 20];
    let mut digits = 0;
    while n > 0 {
        tmp[digits] = b'0' + (n % 10) as u8;
        n /= 10;
        digits += 1;
    }
    let len = digits.min(buf.len());
    for i in 0..len {
        buf[i] = tmp[digits - i - 1];
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> CalculatorState {
        CalculatorState::new()
    }

    fn seq(state: &mut CalculatorState, actions: &[ButtonAction]) {
        for &a in actions {
            state.handle_action(a);
        }
    }

    fn digit(d: u8) -> ButtonAction {
        assert!(d < 10);
        ButtonAction::Digit(d)
    }

    #[test]
    fn test_simple_addition() {
        let mut s = make();
        seq(
            &mut s,
            &[digit(1), ButtonAction::Add, digit(8), ButtonAction::Equals],
        );
        assert_eq!(s.display_str(), "9");
    }

    #[test]
    fn test_chained_operation() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(1),
                ButtonAction::Add,
                digit(8),
                ButtonAction::Subtract,
                digit(4),
                ButtonAction::Equals,
            ],
        );
        assert_eq!(s.display_str(), "5");
    }

    #[test]
    fn test_operator_chaining_by_operator() {
        let mut s = make();
        seq(
            &mut s,
            &[digit(2), ButtonAction::Add, digit(1), ButtonAction::Add],
        );
        assert_eq!(s.display_str(), "3");
        assert_eq!(s.pending_op, Some(Operator::Add));
    }

    #[test]
    fn test_repeated_equals() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(2),
                ButtonAction::Add,
                digit(1),
                ButtonAction::Equals,
                ButtonAction::Equals,
            ],
        );
        assert_eq!(s.display_str(), "4");
    }

    #[test]
    fn test_operator_replacement() {
        let mut s = make();
        seq(
            &mut s,
            &[digit(2), ButtonAction::Add, ButtonAction::Subtract],
        );
        assert_eq!(s.pending_op, Some(Operator::Subtract));
    }

    #[test]
    fn test_decimal_input() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(1),
                ButtonAction::Decimal,
                digit(5),
                ButtonAction::Add,
                digit(2),
                ButtonAction::Equals,
            ],
        );
        assert_eq!(s.display_str(), "3.5");
    }

    #[test]
    fn test_memory_recall() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(5),
                ButtonAction::MemoryAdd,
                ButtonAction::Clear,
                ButtonAction::MemoryRecall,
            ],
        );
        assert_eq!(s.display_str(), "5");
    }

    #[test]
    fn test_memory_subtract() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(10),
                ButtonAction::MemoryAdd,
                digit(3),
                ButtonAction::MemorySubtract,
                ButtonAction::Clear,
                ButtonAction::MemoryRecall,
            ],
        );
        assert_eq!(s.display_str(), "7");
    }

    #[test]
    fn test_square_root() {
        let mut s = make();
        seq(&mut s, &[digit(9), ButtonAction::Sqrt]);
        assert_eq!(s.display_str(), "3");
    }

    #[test]
    fn test_reciprocal() {
        let mut s = make();
        seq(&mut s, &[digit(4), ButtonAction::Reciprocal]);
        assert_eq!(s.display_str(), "0.25");
    }

    #[test]
    fn test_division_by_zero() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(5),
                ButtonAction::Divide,
                digit(0),
                ButtonAction::Equals,
            ],
        );
        assert_eq!(s.display_str(), "Error");
        assert!(s.error);
    }

    #[test]
    fn test_clear() {
        let mut s = make();
        s.memory = 42.0;
        seq(
            &mut s,
            &[digit(9), ButtonAction::Add, digit(1), ButtonAction::Clear],
        );
        assert_eq!(s.display_str(), "0");
        assert_eq!(s.pending_op, None);
        assert_eq!(s.memory, 42.0);
        assert!(!s.error);
    }

    #[test]
    fn test_negate() {
        let mut s = make();
        seq(&mut s, &[digit(5), ButtonAction::Negate]);
        assert_eq!(s.display_str(), "-5");
        seq(&mut s, &[ButtonAction::Negate]);
        assert_eq!(s.display_str(), "5");
    }

    #[test]
    fn test_negate_zero() {
        let mut s = make();
        seq(&mut s, &[ButtonAction::Negate]);
        assert_eq!(s.display_str(), "0");
    }

    #[test]
    fn test_error_clear_on_digit() {
        let mut s = make();
        seq(
            &mut s,
            &[
                digit(5),
                ButtonAction::Divide,
                digit(0),
                ButtonAction::Equals,
            ],
        );
        assert_eq!(s.display_str(), "Error");
        seq(&mut s, &[digit(3)]);
        assert_eq!(s.display_str(), "3");
        assert!(!s.error);
    }

    #[test]
    fn test_percent() {
        let mut s = make();
        seq(
            &mut s,
            &[digit(2), digit(5), digit(0), ButtonAction::Percent],
        );
        assert_eq!(s.display_str(), "2.5");
    }

    #[test]
    fn test_reciprocal_zero() {
        let mut s = make();
        seq(&mut s, &[digit(0), ButtonAction::Reciprocal]);
        assert_eq!(s.display_str(), "Error");
        assert!(s.error);
    }

    #[test]
    fn test_fsqrt() {
        assert!((fsqrt(9.0) - 3.0).abs() < 0.0001);
        assert!((fsqrt(2.0) * fsqrt(2.0) - 2.0).abs() < 0.001);
        assert!((fsqrt(0.0) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_ftrunc() {
        assert_eq!(ftrunc(3.14), 3.0);
        assert_eq!(ftrunc(-3.14), -3.0);
        assert_eq!(ftrunc(0.0), 0.0);
        assert_eq!(ftrunc(100.0), 100.0);
    }

    #[test]
    fn test_val_is_bad() {
        assert!(val_is_bad(f64::NAN));
        assert!(val_is_bad(f64::INFINITY));
        assert!(val_is_bad(f64::NEG_INFINITY));
        assert!(!val_is_bad(0.0));
        assert!(!val_is_bad(1.5));
    }
}
