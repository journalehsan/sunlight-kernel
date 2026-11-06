use super::*;

impl Context {
    pub fn get_property(&mut self, base: Value, name: &str) -> JsResult<Value> {
        self.get(base, &name.encode_utf16().collect())
    }
    pub fn set_property(&mut self, base: Value, name: &str, value: Value) -> JsResult<()> {
        self.put(base, name.encode_utf16().collect(), value)
    }
    /// Missing is distinct from an own property whose value is undefined.
    pub fn has_own(&self, h: Handle, name: &str) -> JsResult<bool> {
        let o = self.object(h)?;
        let k: Key = name.encode_utf16().collect();
        Ok(
            o.props.contains_key(&k)
                || (matches!(o.kind, ObjectKind::Array(_)) && name == "length"),
        )
    }
    pub(super) fn index(k: &Key) -> Option<u32> {
        let s = String::from_utf16(k).ok()?;
        let n = s.parse::<u32>().ok()?;
        (n != u32::MAX && n.to_string() == s).then_some(n)
    }
    pub(super) fn get(&mut self, base: Value, k: &Key) -> JsResult<Value> {
        self.frames.push(Frame::default());
        self.keep_ref(&base);
        let r = self.get_inner(base, k);
        self.frames.pop();
        self.keep_result(&r);
        r
    }
    fn get_inner(&mut self, base: Value, k: &Key) -> JsResult<Value> {
        self.validate(&base)?;
        if *k == "__proto__".encode_utf16().collect::<Key>() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "use Object.getPrototypeOf; legacy __proto__ is unsupported",
            ));
        }
        let length: Key = "length".encode_utf16().collect();
        if let Value::String(s) = &base {
            if *k == length {
                return Ok(Value::Number(s.len() as f64));
            }
            if let Some(i) = Self::index(k) {
                return Ok(s
                    .get(i as usize)
                    .map(|u| Value::String(vec![*u]))
                    .unwrap_or(Value::Undefined));
            }
        }
        let mut h = match base {
            Value::Object(h) => Some(h),
            Value::Null | Value::Undefined => {
                return Err(Error::new(
                    ErrorKind::Type,
                    "property read on null/undefined",
                ))
            }
            _ => Some(self.object_proto),
        };
        while let Some(id) = h {
            self.charge()?;
            let o = self.object(id)?;
            if let Some(p) = o.props.get(k).cloned() {
                return match p {
                    Property::GlobalBinding(n) => self.read(0, &n),
                    Property::Data(v, _, _, _) => Ok(v),
                    Property::Accessor(Some(get), _, _, _) => self.call(get, base, &[]),
                    Property::Accessor(None, _, _, _) => Ok(Value::Undefined),
                };
            }
            h = o.prototype;
        }
        Ok(Value::Undefined)
    }
    pub(super) fn put(&mut self, base: Value, k: Key, value: Value) -> JsResult<()> {
        self.frames.push(Frame::default());
        self.keep_ref(&base);
        self.keep_ref(&value);
        let r = self.put_inner(base, k, value);
        self.frames.pop();

        r
    }
    fn put_inner(&mut self, base: Value, k: Key, value: Value) -> JsResult<()> {
        self.validate(&value)?;
        let h = if let Value::Object(h) = base {
            h
        } else {
            return Err(Error::new(
                ErrorKind::Type,
                "strict property write requires object",
            ));
        };
        self.object(h)?;
        if k == "__proto__".encode_utf16().collect::<Key>() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "legacy prototype mutation unsupported",
            ));
        }
        let mut cursor = Some(h);
        while let Some(id) = cursor {
            self.charge()?;
            let o = self.object(id)?;
            let p = o.props.get(&k).cloned();
            cursor = o.prototype;
            if let Some(p) = p {
                match p {
                    Property::Accessor(_, Some(set), _, _) => {
                        self.call(set, Value::Object(h), &[value])?;
                        return Ok(());
                    }
                    Property::Accessor(_, None, _, _) | Property::Data(_, false, _, _) => {
                        return Err(Error::new(ErrorKind::Type, "property is read-only"))
                    }
                    Property::GlobalBinding(n) => {
                        if id == h {
                            let b = self.envs[0].bindings.get_mut(&n).unwrap();
                            if b.constant {
                                return Err(Error::new(ErrorKind::Type, "property is read-only"));
                            }
                            b.value = Some(value);
                            return Ok(());
                        }
                        break;
                    }
                    Property::Data(_, true, _, _) => break,
                }
            }
        }
        let exists = self.objects[h.1].props.contains_key(&k);
        self.define_descriptor(
            h,
            k,
            PropertyDescriptor {
                value: Some(value),
                writable: (!exists).then_some(true),
                enumerable: (!exists).then_some(true),
                configurable: (!exists).then_some(true),
                ..PropertyDescriptor::default()
            },
        )
    }
}
