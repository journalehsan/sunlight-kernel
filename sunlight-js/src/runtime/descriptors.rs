use super::*;

impl Property {
    fn flags(&self) -> (bool, bool) {
        match self {
            Self::Data(_, _, e, c) | Self::Accessor(_, _, e, c) => (*e, *c),
            Self::GlobalBinding(_) => (true, false),
        }
    }
}
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            (a.is_nan() && b.is_nan()) || (a == b && a.is_sign_negative() == b.is_sign_negative())
        }
        _ => a == b,
    }
}
impl Context {
    pub(super) fn define_descriptor(
        &mut self,
        h: Handle,
        k: Key,
        mut d: PropertyDescriptor,
    ) -> JsResult<()> {
        self.object(h)?;
        let accessor = d.get.is_some() || d.set.is_some();
        let data = d.value.is_some() || d.writable.is_some();
        if accessor && data {
            return Err(Error::new(
                ErrorKind::Type,
                "mixed data/accessor descriptor",
            ));
        }
        for v in [&d.value, &d.get, &d.set].into_iter().flatten() {
            self.validate(v)?;
        }
        for v in [&d.get, &d.set].into_iter().flatten() {
            if *v != Value::Undefined {
                self.callable(v)?;
            }
        }
        // ArraySetLength converts before reading the current descriptor: coercion
        // may reenter the engine and mutate the array.
        if matches!(self.objects[h.1].kind, ObjectKind::Array(_))
            && k == "length".encode_utf16().collect::<Key>()
        {
            if let Some(value) = d.value.take() {
                let first = self.number(value.clone())?;
                let length = if first.is_finite() && first != 0.0 {
                    first.trunc().rem_euclid(4294967296.0) as u32
                } else {
                    0
                };
                let n = self.number(value)?;
                if n != length as f64 {
                    return Err(Error::new(ErrorKind::Range, "invalid array length"));
                }
                d.value = Some(Value::Number(length as f64));
            }
        }
        let old = self.objects[h.1].props.get(&k).cloned();
        if matches!(old, Some(Property::GlobalBinding(_))) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "global binding descriptor redefinition unsupported",
            ));
        }
        let (e, c) = old.as_ref().map(Property::flags).unwrap_or((false, false));
        let old_accessor = matches!(old, Some(Property::Accessor(..)));
        if old.is_some() && !c {
            let invalid = d.configurable == Some(true)
                || d.enumerable.is_some_and(|v| v != e)
                || (accessor && !old_accessor)
                || (data && old_accessor);
            let changed = match &old {
                Some(Property::Data(v, false, _, _)) => {
                    d.writable == Some(true) || d.value.as_ref().is_some_and(|n| !same(v, n))
                }
                Some(Property::Accessor(g, s, _, _)) => {
                    d.get
                        .as_ref()
                        .is_some_and(|v| !same(g.as_ref().unwrap_or(&Value::Undefined), v))
                        || d.set
                            .as_ref()
                            .is_some_and(|v| !same(s.as_ref().unwrap_or(&Value::Undefined), v))
                }
                _ => false,
            };
            if invalid || changed {
                return Err(Error::new(ErrorKind::Type, "non-configurable property"));
            }
        }
        let e = d.enumerable.unwrap_or(e);
        let c = d.configurable.unwrap_or(c);
        let p = if accessor || (!data && old_accessor) {
            let (g, s) = match old.clone() {
                Some(Property::Accessor(g, s, _, _)) => (g, s),
                _ => (None, None),
            };
            let callback = |v: Option<Value>, old| match v {
                Some(Value::Undefined) => None,
                Some(v) => Some(v),
                None => old,
            };
            Property::Accessor(callback(d.get, g), callback(d.set, s), e, c)
        } else {
            let (v, w) = match old.clone() {
                Some(Property::Data(v, w, _, _)) => (v, w),
                _ => (Value::Undefined, false),
            };
            Property::Data(d.value.unwrap_or(v), d.writable.unwrap_or(w), e, c)
        };
        if let ObjectKind::Array(len) = self.objects[h.1].kind {
            let length_key: Key = "length".encode_utf16().collect();
            if k == length_key {
                let Property::Data(value, writable, _, _) = &p else {
                    return Err(Error::new(ErrorKind::Type, "array length must be data"));
                };
                let Value::Number(n) = value else {
                    unreachable!("normalized array length")
                };
                let n = *n as u32;
                let mut keys: Vec<_> = self.objects[h.1]
                    .props
                    .keys()
                    .filter_map(|key| {
                        Self::index(key)
                            .filter(|i| *i >= n)
                            .map(|i| (i, key.clone()))
                    })
                    .collect();
                keys.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                for (i, key) in keys {
                    self.charge()?;
                    if !self.objects[h.1].props[&key].flags().1 {
                        self.objects[h.1].kind = ObjectKind::Array(i + 1);
                        self.objects[h.1].props.insert(
                            length_key,
                            Property::Data(Value::Number((i + 1) as f64), *writable, false, false),
                        );
                        return Err(Error::new(
                            ErrorKind::Type,
                            "non-configurable array element",
                        ));
                    }
                    self.objects[h.1].props.remove(&key);
                }
                self.objects[h.1].kind = ObjectKind::Array(n);
                self.objects[h.1].props.insert(
                    k,
                    Property::Data(Value::Number(n as f64), *writable, false, false),
                );
                return Ok(());
            }
            if let Some(i) = Self::index(&k) {
                if i >= len {
                    if matches!(
                        self.objects[h.1].props.get(&length_key),
                        Some(Property::Data(_, false, _, _))
                    ) {
                        return Err(Error::new(ErrorKind::Type, "array length is read-only"));
                    }
                    if old.is_none() {
                        self.capacity()?;
                    }
                    self.objects[h.1].kind = ObjectKind::Array(i + 1);
                    self.objects[h.1].props.insert(
                        length_key,
                        Property::Data(Value::Number((i + 1) as f64), true, false, false),
                    );
                }
            }
        }
        if old.is_none() {
            self.capacity()?;
            self.entries += 1;
        }
        self.objects[h.1].props.insert(k, p);
        Ok(())
    }
    pub(super) fn descriptor_object(&mut self, h: Handle, k: &Key) -> JsResult<Value> {
        let Some(p) = self.object(h)?.props.get(k).cloned() else {
            return Ok(Value::Undefined);
        };
        let out = self.create_object(Some(self.object_proto))?;
        let (e, c) = p.flags();
        let fields = match p {
            Property::Data(v, w, _, _) => vec![("value", v), ("writable", Value::Boolean(w))],
            Property::Accessor(g, s, _, _) => vec![
                ("get", g.unwrap_or(Value::Undefined)),
                ("set", s.unwrap_or(Value::Undefined)),
            ],
            Property::GlobalBinding(n) => vec![
                ("value", self.read(0, &n)?),
                (
                    "writable",
                    Value::Boolean(!self.envs[0].bindings[&n].constant),
                ),
            ],
        };
        for (key, value) in fields.into_iter().chain([
            ("enumerable", Value::Boolean(e)),
            ("configurable", Value::Boolean(c)),
        ]) {
            self.literal_property(out, key.encode_utf16().collect(), value)?;
        }
        Ok(Value::Object(out))
    }
    pub(super) fn has_property(&mut self, mut h: Handle, k: &Key) -> JsResult<bool> {
        loop {
            self.charge()?;
            let o = self.object(h)?;
            if o.props.contains_key(k) {
                return Ok(true);
            }
            match o.prototype {
                Some(p) => h = p,
                None => return Ok(false),
            }
        }
    }
}
