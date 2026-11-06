use super::*;

impl Context {
    pub fn create_object(&mut self, prototype: Option<Handle>) -> JsResult<Handle> {
        self.alloc(ObjectKind::Ordinary, prototype)
    }
    /// Installs a non-configurable, enumerable host data property.
    pub fn define_data(
        &mut self,
        h: Handle,
        name: &str,
        value: Value,
        writable: bool,
    ) -> JsResult<()> {
        self.define_property(
            h,
            name,
            PropertyDescriptor {
                value: Some(value),
                writable: Some(writable),
                enumerable: Some(true),
                configurable: Some(false),
                ..PropertyDescriptor::default()
            },
        )
    }
    /// Native callbacks are created with `native_function` and receive the receiver.
    pub fn define_accessor(
        &mut self,
        h: Handle,
        name: &str,
        get: Option<Value>,
        set: Option<Value>,
    ) -> JsResult<()> {
        self.define_property(
            h,
            name,
            PropertyDescriptor {
                get: Some(get.unwrap_or(Value::Undefined)),
                set: Some(set.unwrap_or(Value::Undefined)),
                enumerable: Some(true),
                configurable: Some(false),
                ..PropertyDescriptor::default()
            },
        )
    }
    pub fn define_property(
        &mut self,
        h: Handle,
        name: &str,
        descriptor: PropertyDescriptor,
    ) -> JsResult<()> {
        self.frames.push(Frame::default());
        self.keep(Value::Object(h));
        for v in descriptor
            .value
            .iter()
            .chain(descriptor.get.iter())
            .chain(descriptor.set.iter())
        {
            self.keep_ref(v);
        }
        let result = self.define_descriptor(h, name.encode_utf16().collect(), descriptor);
        self.frames.pop();
        result
    }
    pub fn set_global(&mut self, name: &str, value: Value) -> JsResult<()> {
        self.check()?;
        self.validate(&value)?;
        if self.envs[0].bindings.contains_key(name) {
            return Err(Error::new(ErrorKind::Type, "global binding already exists"));
        }
        self.capacity()?;
        self.entries += 1;
        self.envs[0].bindings.insert(
            name.into(),
            Binding {
                value: Some(value),
                constant: true,
            },
        );
        self.capacity()?;
        self.entries += 1;
        self.objects[self.global.1].props.insert(
            name.encode_utf16().collect(),
            Property::GlobalBinding(name.into()),
        );
        Ok(())
    }
    pub fn native_function<F>(&mut self, f: F) -> JsResult<Value>
    where
        F: Fn(&mut Context, Value, &[Value]) -> JsResult<Value> + 'static,
    {
        Ok(Value::Object(self.alloc(
            ObjectKind::Native(Rc::new(f), Vec::new()),
            Some(self.function_proto),
        )?))
    }
    /// Declare every JS value captured by an opaque Rust callback. These edges
    /// are traced only while the native function is reachable.
    pub fn native_function_with_roots<F>(&mut self, captures: Vec<Value>, f: F) -> JsResult<Value>
    where
        F: Fn(&mut Context, Value, &[Value]) -> JsResult<Value> + 'static,
    {
        for value in &captures {
            self.validate(value)?;
        }
        Ok(Value::Object(self.alloc(
            ObjectKind::Native(Rc::new(f), captures),
            Some(self.function_proto),
        )?))
    }
    pub fn register_native<F>(&mut self, name: &str, f: F) -> JsResult<()>
    where
        F: Fn(&mut Context, Value, &[Value]) -> JsResult<Value> + 'static,
    {
        let v = self.native_function(f)?;
        self.set_global(name, v)
    }
}
