use super::*;

#[cfg(test)]
mod tests;

const MAX_SESSIONS: usize = 16;

/// A host fixture stands in for the kernel's authenticated registration
/// binding. Only the trusted coordinator may issue or withdraw this handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub(super) registration: Registration,
    key: Key,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media(Key);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub blocks: u64,
    pub block_bytes: u64,
}

impl Geometry {
    fn capacity(self) -> Option<u64> {
        if self.blocks == 0
            || !self.block_bytes.is_power_of_two()
            || self.block_bytes > BLOCK_SIZE as u64
        {
            return None;
        }
        self.blocks.checked_mul(self.block_bytes)
    }
}

/// Byte range on the canonical device. Offsets are absolute, not relative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub length: u64,
}

impl Extent {
    fn end(self) -> Result<u64, Error> {
        self.offset
            .checked_add(self.length)
            .ok_or(Error::OutOfRange)
    }

    fn validate(self, geometry: Geometry) -> Result<(), Error> {
        if self.end()? > geometry.capacity().ok_or(Error::InvalidGeometry)?
            || self.length == 0
            || self.offset % geometry.block_bytes != 0
            || self.length % geometry.block_bytes != 0
        {
            return Err(Error::OutOfRange);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights {
    pub read: bool,
    pub write: bool,
    pub flush: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Grant {
    pub(super) session: Session,
    extent: Extent,
    rights: Rights,
}

pub(super) struct Medium {
    pub(super) key: Key,
    geometry: Geometry,
}

#[derive(Default)]
pub(super) struct Admission {
    sessions: BTreeMap<u64, Session>,
    pub(super) media: Option<Medium>,
}

impl Admission {
    pub(super) fn check_session(&self, session: &Session) -> Result<(), Error> {
        if self.sessions.get(&session.registration.owner.endpoint) == Some(session) {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    fn geometry(&self, media: &Media) -> Result<Geometry, Error> {
        let registered = self.media.as_ref().ok_or(Error::NoMedia)?;
        if registered.key != media.0 {
            return Err(Error::Stale);
        }
        Ok(registered.geometry)
    }

    fn grant(
        &self,
        session: &Session,
        geometry: Geometry,
        extent: Extent,
        rights: Rights,
    ) -> Result<Grant, Error> {
        self.check_session(session)?;
        extent.validate(geometry)?;
        if rights.flush && (extent.offset != 0 || Some(extent.length) != geometry.capacity()) {
            return Err(Error::Unauthorized);
        }
        Ok(Grant {
            session: session.clone(),
            extent,
            rights,
        })
    }
}

impl State {
    fn authenticate(&self, session: &Session, lease: &Lease) -> Result<(&Grant, Geometry), Error> {
        self.check_lease(lease)?;
        self.admission.check_session(session)?;
        let grant = self
            .lease
            .as_ref()
            .and_then(|lease| lease.grant.as_ref())
            .filter(|grant| &grant.session == session)
            .ok_or(Error::Unauthorized)?;
        if !matches!(self.phase, Phase::Active) {
            return Err(Error::AdmissionClosed);
        }
        let medium = self.admission.media.as_ref().ok_or(Error::NoMedia)?;
        if Some(&medium.key) != lease.media.as_ref() {
            return Err(Error::Stale);
        }
        Ok((grant, medium.geometry))
    }
}

impl DeviceLease {
    /// Trusted host control plane. The input must come from the authenticated
    /// registration authority, never from operation payloads.
    pub fn bind_registration(&self, registration: Registration) -> Result<Session, Error> {
        let mut state = self.state.lock();
        let owner = registration.owner;
        if [
            owner.pid,
            owner.incarnation,
            owner.address_space_generation,
            registration.epoch,
        ]
        .contains(&0)
        {
            return Err(Error::Unauthorized);
        }
        if state.admission.sessions.contains_key(&owner.endpoint) {
            return Err(Error::Occupied);
        }
        if state.admission.sessions.len() >= MAX_SESSIONS {
            return Err(Error::NoCredit);
        }
        let session = Session {
            registration,
            key: state.key()?,
        };
        state
            .admission
            .sessions
            .insert(owner.endpoint, session.clone());
        Ok(session)
    }

    /// Close admission immediately. The coordinator still owns drain and
    /// retirement through the incumbent lease, even after its session dies.
    pub fn withdraw_registration(&self, session: &Session) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.admission.check_session(session)?;
        state
            .admission
            .sessions
            .remove(&session.registration.owner.endpoint);
        Ok(())
    }

    /// Register a fresh blank incarnation of the same fake device slot.
    pub fn register_media(&self, geometry: Geometry) -> Result<Media, Error> {
        let mut state = self.state.lock();
        geometry.capacity().ok_or(Error::InvalidGeometry)?;
        #[cfg(not(target_os = "none"))]
        if geometry.blocks > state.transport.visible.len() as u64 {
            return Err(Error::InvalidGeometry);
        }
        match state.phase {
            Phase::Vacant if state.admission.media.is_none() => {}
            Phase::Fenced(_) => return Err(Error::Fenced),
            _ => return Err(Error::Occupied),
        }
        let key = state.key()?;
        #[cfg(not(target_os = "none"))]
        {
            state.transport.visible.fill([0; BLOCK_SIZE]);
            state.transport.durable.fill([0; BLOCK_SIZE]);
        }
        state.admission.media = Some(Medium {
            key: key.clone(),
            geometry,
        });
        Ok(Media(key))
    }

    /// An unplanned removal closes admission and keeps the original transport
    /// and outstanding commands fenced. Planned replacement requires a
    /// successful release or retirement before physical removal.
    pub fn remove_media(&self, media: &Media) -> Result<(), Error> {
        let mut state = self.state.lock();
        state.admission.geometry(media)?;
        state.admission.media = None;
        if !matches!(state.phase, Phase::Vacant) {
            state.fence(Fence::MediaRemoved);
        }
        Ok(())
    }

    pub fn acquire_authenticated(
        &self,
        media: &Media,
        session: &Session,
        extent: Extent,
        rights: Rights,
    ) -> Result<Lease, Error> {
        let mut state = self.state.lock();
        if matches!(state.phase, Phase::Fenced(_)) {
            return Err(Error::Fenced);
        }
        let geometry = state.admission.geometry(media)?;
        let grant = state.admission.grant(session, geometry, extent, rights)?;
        if !matches!(state.phase, Phase::Vacant) {
            return Err(Error::Occupied);
        }
        let key = state.key()?;
        Ok(state.activate_grant(session.registration, key, Some(grant)))
    }

    pub fn read_authenticated(
        &self,
        session: &Session,
        lease: &Lease,
        range: Extent,
    ) -> Result<Operation, Error> {
        self.submit_authenticated(session, lease, range, None)
    }

    /// The first logical block's worth of bytes is the write payload. The
    /// rest of the fixed-size host buffer is padding and is discarded.
    pub fn write_authenticated(
        &self,
        session: &Session,
        lease: &Lease,
        range: Extent,
        bytes: Block,
    ) -> Result<Operation, Error> {
        self.submit_authenticated(session, lease, range, Some(bytes))
    }

    fn submit_authenticated(
        &self,
        session: &Session,
        lease: &Lease,
        range: Extent,
        bytes: Option<Block>,
    ) -> Result<Operation, Error> {
        let mut state = self.state.lock();
        let (grant, geometry) = state.authenticate(session, lease)?;
        if !(if bytes.is_some() {
            grant.rights.write
        } else {
            grant.rights.read
        }) {
            return Err(Error::Unauthorized);
        }
        range.validate(geometry)?;
        if range.length != geometry.block_bytes
            || range.offset < grant.extent.offset
            || range.end()? > grant.extent.end()?
        {
            return Err(Error::OutOfRange);
        }
        let block = (range.offset / geometry.block_bytes) as usize;
        let command = match bytes {
            Some(mut bytes) => {
                bytes[geometry.block_bytes as usize..].fill(0);
                Command::Write(block, bytes)
            }
            None => Command::Read(block),
        };
        state.enqueue(command)
    }

    /// Close data admission for a client-initiated whole-device barrier. The
    /// coordinator drives drain and terminal settlement using the N1E API.
    pub fn request_flush(&self, session: &Session, lease: &Lease) -> Result<Transition, Error> {
        let mut state = self.state.lock();
        let (grant, _) = state.authenticate(session, lease)?;
        if !grant.rights.flush {
            return Err(Error::Unauthorized);
        }
        state.begin(lease, Destination::ClientFlush)
    }

    #[cfg(not(target_os = "none"))]
    pub(crate) fn with_attachment_pin<T>(
        &self,
        session: &Session,
        lease: &Lease,
        publish: impl FnOnce() -> T,
    ) -> Result<T, Error> {
        let state = self.state.lock();
        state.authenticate(session, lease)?;
        let result = publish();
        drop(state);
        Ok(result)
    }

    #[cfg(not(target_os = "none"))]
    pub(crate) fn finish_attachment_release<E>(
        &self,
        transition: &Transition,
        remove: impl FnOnce() -> Result<(), E>,
    ) -> Result<Result<(), E>, Error> {
        let mut state = self.state.lock();
        match &state.phase {
            Phase::Synchronized(change)
                if change.key == transition.0
                    && matches!(change.destination, Destination::Release) => {}
            Phase::Fenced(_) => return Err(Error::Fenced),
            _ => return Err(Error::Stale),
        }
        let result = remove();
        if result.is_ok() {
            state.release();
        }
        Ok(result)
    }

    #[cfg(not(target_os = "none"))]
    pub(crate) fn attachment_blocks(&self, media: &Media) -> Result<u64, Error> {
        let geometry = self.state.lock().admission.geometry(media)?;
        if geometry.block_bytes != BLOCK_SIZE as u64 {
            return Err(Error::InvalidGeometry);
        }
        Ok(geometry.blocks)
    }

    #[cfg(not(target_os = "none"))]
    pub fn disposable_image(image: Box<[Block]>) -> Result<(Self, Media), Error> {
        if image.is_empty() || image.len() > 8192 {
            return Err(Error::InvalidGeometry);
        }
        let device = Self::new();
        let geometry = Geometry {
            blocks: image.len() as u64,
            block_bytes: BLOCK_SIZE as u64,
        };
        {
            let mut state = device.state.lock();
            state.transport.visible = image.clone();
            state.transport.durable = image;
        }
        let media = {
            let mut state = device.state.lock();
            let key = state.key()?;
            state.admission.media = Some(Medium {
                key: key.clone(),
                geometry,
            });
            Media(key)
        };
        Ok((device, media))
    }

    /// The trusted coordinator supplies the successor's authenticated
    /// binding and authorized range/rights. Commit rechecks binding liveness.
    pub fn begin_authenticated_transfer(
        &self,
        session: &Session,
        lease: &Lease,
        successor: &Session,
        extent: Extent,
        rights: Rights,
    ) -> Result<Transition, Error> {
        let mut state = self.state.lock();
        let (_, geometry) = state.authenticate(session, lease)?;
        if session == successor {
            return Err(Error::InvalidSuccessor);
        }
        let grant = state.admission.grant(successor, geometry, extent, rights)?;
        state.begin(lease, Destination::AuthenticatedTransfer(grant))
    }
}
