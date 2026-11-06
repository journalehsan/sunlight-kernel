use super::*;

impl Context {
    pub(super) fn primitive(&mut self, v: Value, string_hint: bool) -> JsResult<Value> {
        if !matches!(v, Value::Object(_)) {
            return Ok(v);
        }
        for name in if string_hint {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        } {
            let method = self.get_property(v.clone(), name)?;
            if self.callable(&method).is_ok() {
                let result = self.call(method, v.clone(), &[])?;
                if !matches!(result, Value::Object(_)) {
                    return Ok(result);
                }
            }
        }
        Err(Error::new(
            ErrorKind::Type,
            "object coercion did not produce a primitive",
        ))
    }
    pub(super) fn key(&mut self, v: Value) -> JsResult<Key> {
        let p = self.primitive(v, true)?;
        Ok(match p {
            Value::String(s) => s,
            _ => p.display().encode_utf16().collect(),
        })
    }
    pub(super) fn number(&mut self, v: Value) -> JsResult<f64> {
        let v = self.primitive(v, false)?;
        Ok(match v {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Boolean(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Number(n) => n,
            Value::String(s) => {
                let s = String::from_utf16_lossy(&s);
                let t = s.trim_matches(lexer::space);
                if t.is_empty() {
                    0.0
                } else if ["Infinity", "+Infinity"].contains(&t) {
                    f64::INFINITY
                } else if t == "-Infinity" {
                    -f64::INFINITY
                } else if t.starts_with("0x")
                    || t.starts_with("0X")
                    || t.starts_with("0b")
                    || t.starts_with("0B")
                    || t.starts_with("0o")
                    || t.starts_with("0O")
                {
                    let radix = match t.as_bytes()[1].to_ascii_lowercase() {
                        b'x' => 16,
                        b'b' => 2,
                        _ => 8,
                    };
                    if t.len() == 2 || !t[2..].chars().all(|c| c.is_digit(radix)) {
                        f64::NAN
                    } else {
                        match u128::from_str_radix(&t[2..], radix) {
                            Ok(n) => n as f64,
                            Err(_) => {
                                return Err(Error::new(
                                    ErrorKind::Unsupported,
                                    "nondecimal numeric string exceeds 128-bit conversion support",
                                ))
                            }
                        }
                    }
                } else if t.chars().all(|c| c.is_ascii_digit() || ".eE+-".contains(c)) {
                    t.parse().unwrap_or(f64::NAN)
                } else {
                    f64::NAN
                }
            }
            _ => unreachable!(),
        })
    }
    pub(super) fn binary(&mut self, op: &str, a: Value, b: Value) -> JsResult<Value> {
        if op == "===" || op == "!==" {
            return Ok(Value::Boolean(if op == "===" { a == b } else { a != b }));
        }
        let a = self.primitive(a, false)?;
        let b = self.primitive(b, false)?;
        if op == "+" && (matches!(a, Value::String(_)) || matches!(b, Value::String(_))) {
            let mut s = match a {
                Value::String(s) => s,
                _ => a.display().encode_utf16().collect(),
            };
            let tail = match b {
                Value::String(s) => s,
                _ => b.display().encode_utf16().collect::<Key>(),
            };
            if s.len().saturating_add(tail.len()) > self.limits.string_units {
                return Err(Error::new(ErrorKind::Limit, "string size limit"));
            }
            s.extend(tail);
            return Ok(Value::String(s));
        }
        if matches!(op, "<" | ">" | "<=" | ">=") {
            if let (Value::String(a), Value::String(b)) = (&a, &b) {
                return Ok(Value::Boolean(match op {
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    _ => a >= b,
                }));
            }
        }
        let a = self.number(a)?;
        let b = self.number(b)?;
        Ok(match op {
            "+" => Value::Number(a + b),
            "-" => Value::Number(a - b),
            "*" => Value::Number(a * b),
            "/" => Value::Number(a / b),
            "%" => Value::Number(a % b),
            "<" => Value::Boolean(a < b),
            ">" => Value::Boolean(a > b),
            "<=" => Value::Boolean(a <= b),
            ">=" => Value::Boolean(a >= b),
            _ => unreachable!(),
        })
    }
}
