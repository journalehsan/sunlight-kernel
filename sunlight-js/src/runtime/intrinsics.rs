use super::*;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}
fn handle(v: Value) -> JsResult<Handle> {
    match v {
        Value::Object(h) => Ok(h),
        _ => Err(Error::new(ErrorKind::Type, "object required")),
    }
}
impl Context {
    fn method<F>(&mut self, h: Handle, name: &str, f: F) -> JsResult<()>
    where
        F: Fn(&mut Context, Value, &[Value]) -> JsResult<Value> + 'static,
    {
        let v = self.native_function(f)?;
        self.define_property(
            h,
            name,
            PropertyDescriptor {
                value: Some(v),
                writable: Some(true),
                configurable: Some(true),
                enumerable: Some(false),
                ..PropertyDescriptor::default()
            },
        )
    }
    pub(super) fn install_intrinsics(&mut self) -> JsResult<()> {
        self.objects[self.array_proto.1].kind = ObjectKind::Array(0);
        self.objects[self.array_proto.1].props.insert(
            "length".encode_utf16().collect(),
            Property::Data(Value::Number(0.0), true, false, false),
        );
        self.entries += 1;
        self.objects[self.function_proto.1].kind =
            ObjectKind::Native(Rc::new(|_, _, _| Ok(Value::Undefined)), Vec::new());
        self.method(self.object_proto, "hasOwnProperty", |c, this, args| {
            if matches!(this, Value::Null | Value::Undefined) {
                return Err(Error::new(ErrorKind::Type, "null receiver"));
            }
            let k = c.key(arg(args, 0))?;
            let yes = match this {
                Value::Object(h) => c.object(h)?.props.contains_key(&k),
                Value::String(s) => {
                    k == "length".encode_utf16().collect::<Key>()
                        || Self::index(&k).is_some_and(|i| (i as usize) < s.len())
                }
                _ => false,
            };
            Ok(Value::Boolean(yes))
        })?;
        self.method(self.object_proto, "toString", |c, this, _| {
            let tag = match this {
                Value::Undefined => "Undefined",
                Value::Null => "Null",
                Value::Boolean(_) => "Boolean",
                Value::Number(_) => "Number",
                Value::String(_) => "String",
                Value::Object(h) => match c.object(h)?.kind {
                    ObjectKind::Array(_) => "Array",
                    ObjectKind::Function(_) | ObjectKind::Native(..) => "Function",
                    _ => "Object",
                },
            };
            Ok(Value::string(&format!("[object {tag}]")))
        })?;
        self.method(self.object_proto, "valueOf", |_, this, _| Ok(this))?;
        self.method(self.function_proto, "call", |c, this, args| {
            c.call(this, arg(args, 0), args.get(1..).unwrap_or(&[]))
        })?;
        self.method(self.function_proto, "apply", |c, this, args| {
            c.callable(&this)?;
            let list = arg(args, 1);
            let mut values = Vec::new();
            if !matches!(list, Value::Null | Value::Undefined) {
                handle(list.clone())?;
                let len = c.array_like_length(list.clone())?;
                for i in 0..len {
                    c.charge()?;
                    values.push(c.get_property(list.clone(), &i.to_string())?);
                }
            }
            c.call(this, arg(args, 0), &values)
        })?;
        self.method(self.array_proto, "push", |c, this, args| {
            handle(this.clone())?;
            let len = c.array_like_length(this.clone())?;
            if len + args.len() as u64 > 9007199254740991 {
                return Err(Error::new(ErrorKind::Type, "array-like length overflow"));
            }
            for (i, v) in args.iter().enumerate() {
                c.charge()?;
                c.set_property(this.clone(), &(len + i as u64).to_string(), v.clone())?;
            }
            let n = Value::Number((len + args.len() as u64) as f64);
            c.set_property(this, "length", n.clone())?;
            Ok(n)
        })?;
        self.method(self.array_proto, "pop", |c, this, _| {
            let h = handle(this.clone())?;
            let len = c.array_like_length(this.clone())?;
            if len == 0 {
                c.set_property(this, "length", Value::Number(0.0))?;
                return Ok(Value::Undefined);
            }
            let key = (len - 1).to_string();
            let value = c.get_property(this.clone(), &key)?;
            let k: Key = key.encode_utf16().collect();
            if let Some(p) = c.objects[h.1].props.get(&k) {
                let configurable = match p {
                    Property::Data(_, _, _, c) | Property::Accessor(_, _, _, c) => *c,
                    _ => false,
                };
                if !configurable {
                    return Err(Error::new(
                        ErrorKind::Type,
                        "cannot remove non-configurable element",
                    ));
                }
            }
            c.objects[h.1].props.remove(&k);
            c.set_property(this, "length", Value::Number((len - 1) as f64))?;
            Ok(value)
        })?;
        self.method(self.array_proto, "slice", |c, this, args| {
            let h = handle(this.clone())?;
            let len = c.array_like_length(this.clone())?;
            let start = c.relative_index(arg(args, 0), len)?;
            let end = if arg(args, 1) == Value::Undefined {
                len
            } else {
                c.relative_index(arg(args, 1), len)?
            };
            let out = c.alloc(ObjectKind::Array(0), Some(c.array_proto))?;
            for i in start..end {
                c.charge()?;
                let key: Key = i.to_string().encode_utf16().collect();
                if c.has_property(h, &key)? {
                    let v = c.get(this.clone(), &key)?;
                    c.literal_property(out, (i - start).to_string().encode_utf16().collect(), v)?;
                }
            }
            c.set_property(
                Value::Object(out),
                "length",
                Value::Number(end.saturating_sub(start) as f64),
            )?;
            Ok(Value::Object(out))
        })?;
        let object = handle(self.native_function(|c, _, args| match arg(args, 0) {
            v @ Value::Object(_) => Ok(v),
            Value::Null | Value::Undefined => {
                Ok(Value::Object(c.create_object(Some(c.object_proto))?))
            }
            _ => Err(Error::new(
                ErrorKind::Unsupported,
                "boxed primitives unsupported",
            )),
        })?)?;
        let array = handle(self.native_function(|c, _, args| {
            let h = c.alloc(ObjectKind::Array(0), Some(c.array_proto))?;
            if args.len() == 1 && matches!(args[0], Value::Number(_)) {
                c.set_property(Value::Object(h), "length", args[0].clone())?;
            } else {
                for (i, v) in args.iter().enumerate() {
                    c.literal_property(h, i.to_string().encode_utf16().collect(), v.clone())?;
                }
            }
            Ok(Value::Object(h))
        })?)?;
        let function = handle(self.native_function(|_, _, _| {
            Err(Error::new(
                ErrorKind::Unsupported,
                "dynamic Function constructor unsupported",
            ))
        })?)?;
        for (name, ctor, proto) in [
            ("Object", object, self.object_proto),
            ("Array", array, self.array_proto),
            ("Function", function, self.function_proto),
        ] {
            self.define_property(
                ctor,
                "prototype",
                PropertyDescriptor {
                    value: Some(Value::Object(proto)),
                    ..PropertyDescriptor::default()
                },
            )?;
            self.define_property(
                proto,
                "constructor",
                PropertyDescriptor {
                    value: Some(Value::Object(ctor)),
                    writable: Some(true),
                    configurable: Some(true),
                    ..PropertyDescriptor::default()
                },
            )?;
            self.set_global(name, Value::Object(ctor))?;
        }
        self.method(object, "create", |c, _, args| {
            if args.get(1).is_some_and(|v| *v != Value::Undefined) {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "Object.create descriptors argument unsupported",
                ));
            }
            let proto = match arg(args, 0) {
                Value::Null => None,
                Value::Object(h) => Some(h),
                _ => return Err(Error::new(ErrorKind::Type, "invalid prototype")),
            };
            Ok(Value::Object(c.create_object(proto)?))
        })?;
        self.method(object, "getPrototypeOf", |c, _, args| {
            let h = handle(arg(args, 0))?;
            Ok(c.object(h)?
                .prototype
                .map(Value::Object)
                .unwrap_or(Value::Null))
        })?;
        self.method(object, "getOwnPropertyDescriptor", |c, _, args| {
            let h = handle(arg(args, 0))?;
            let k = c.key(arg(args, 1))?;
            c.descriptor_object(h, &k)
        })?;
        self.method(object, "defineProperty", |c, _, args| {
            let h = handle(arg(args, 0))?;
            c.object(h)?;
            let k = c.key(arg(args, 1))?;
            let source = arg(args, 2);
            let dh = handle(source.clone())?;
            let mut d = PropertyDescriptor::default();
            // ToPropertyDescriptor observes inherited fields and their getters in order.
            for name in [
                "enumerable",
                "configurable",
                "value",
                "writable",
                "get",
                "set",
            ] {
                if c.has_property(dh, &name.encode_utf16().collect())? {
                    let v = c.get_property(source.clone(), name)?;
                    match name {
                        "enumerable" => d.enumerable = Some(v.truthy()),
                        "configurable" => d.configurable = Some(v.truthy()),
                        "value" => d.value = Some(v),
                        "writable" => d.writable = Some(v.truthy()),
                        "get" | "set" => {
                            if v != Value::Undefined {
                                c.callable(&v)?;
                            }
                            if name == "get" {
                                d.get = Some(v);
                            } else {
                                d.set = Some(v);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
            c.define_descriptor(h, k, d)?;
            Ok(Value::Object(h))
        })?;
        Ok(())
    }
    fn array_like_length(&mut self, this: Value) -> JsResult<u64> {
        let v = self.get_property(this, "length")?;
        let n = self.number(v)?;
        Ok(if n.is_nan() || n <= 0.0 {
            0
        } else {
            n.min(9007199254740991.0).floor() as u64
        })
    }
    fn relative_index(&mut self, v: Value, len: u64) -> JsResult<u64> {
        let n = self.number(v)?;
        let n = if n.is_nan() { 0.0 } else { n.trunc() };
        Ok(if n < 0.0 {
            (len as f64 + n).max(0.0) as u64
        } else {
            n.min(len as f64) as u64
        })
    }
}
