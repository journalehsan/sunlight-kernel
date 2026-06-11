use alloc::vec::Vec;

/// Signal numbers (POSIX-style)
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    SIGHUP = 1,
    SIGINT = 2,     // Ctrl+C
    SIGQUIT = 3,    // Ctrl+\
    SIGILL = 4,
    SIGTRAP = 5,
    SIGABRT = 6,
    SIGBUS = 7,
    SIGFPE = 8,
    SIGKILL = 9,    // cannot be caught
    SIGUSR1 = 10,
    SIGSEGV = 11,
    SIGUSR2 = 12,
    SIGPIPE = 13,
    SIGALRM = 14,
    SIGTERM = 15,   // termination signal
    SIGCHLD = 17,   // child stopped or terminated
    SIGCONT = 18,   // continue stopped process
    SIGSTOP = 19,   // cannot be caught
    SIGTSTP = 20,   // Ctrl+Z
    SIGTTIN = 21,
    SIGTTOU = 22,
    SIGWINCH = 28,  // terminal window size changed
}

impl Signal {
    pub fn try_from_u32(n: u32) -> Option<Self> {
        match n {
            1 => Some(Signal::SIGHUP),
            2 => Some(Signal::SIGINT),
            3 => Some(Signal::SIGQUIT),
            4 => Some(Signal::SIGILL),
            5 => Some(Signal::SIGTRAP),
            6 => Some(Signal::SIGABRT),
            7 => Some(Signal::SIGBUS),
            8 => Some(Signal::SIGFPE),
            9 => Some(Signal::SIGKILL),
            10 => Some(Signal::SIGUSR1),
            11 => Some(Signal::SIGSEGV),
            12 => Some(Signal::SIGUSR2),
            13 => Some(Signal::SIGPIPE),
            14 => Some(Signal::SIGALRM),
            15 => Some(Signal::SIGTERM),
            17 => Some(Signal::SIGCHLD),
            18 => Some(Signal::SIGCONT),
            19 => Some(Signal::SIGSTOP),
            20 => Some(Signal::SIGTSTP),
            21 => Some(Signal::SIGTTIN),
            22 => Some(Signal::SIGTTOU),
            28 => Some(Signal::SIGWINCH),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// How a signal is handled
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigHandler {
    Default,           // SIG_DFL: default action
    Ignore,            // SIG_IGN: ignore the signal
    UserHandler(u64),  // function pointer in user-space
}

/// Signal action
#[derive(Clone, Copy, Debug)]
pub struct SigAction {
    pub handler: SigHandler,
    pub mask: u64,     // signals to block during handler execution
    pub flags: u32,    // SA_RESTART, SA_SIGINFO, SA_NOCLDWAIT, etc.
}

impl SigAction {
    pub fn default() -> Self {
        Self {
            handler: SigHandler::Default,
            mask: 0,
            flags: 0,
        }
    }
}

/// Default signal actions
pub fn get_default_action(signal: Signal) -> SigHandler {
    match signal {
        Signal::SIGKILL | Signal::SIGSTOP => SigHandler::Default,  // Cannot be caught
        Signal::SIGCHLD | Signal::SIGWINCH => SigHandler::Ignore,
        Signal::SIGCONT => SigHandler::Default,  // Resume process
        _ => SigHandler::Default,  // Terminate by default
    }
}

/// Errors from signal operations
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalError {
    InvalidSignal,
    InvalidHandler,
    CannotCatch,  // SIGKILL and SIGSTOP cannot be caught
    PermissionDenied,
}

/// Signal mask (set of signals)
#[derive(Clone, Copy, Debug)]
pub struct SignalMask {
    bits: u64,
}

impl SignalMask {
    pub fn new() -> Self {
        Self { bits: 0 }
    }

    pub fn add(&mut self, signal: Signal) {
        self.bits |= 1 << (signal.as_u32() - 1);
    }

    pub fn remove(&mut self, signal: Signal) {
        self.bits &= !(1 << (signal.as_u32() - 1));
    }

    pub fn contains(&self, signal: Signal) -> bool {
        (self.bits & (1 << (signal.as_u32() - 1))) != 0
    }

    pub fn bits(&self) -> u64 {
        self.bits
    }

    pub fn set_bits(&mut self, bits: u64) {
        self.bits = bits;
    }
}

impl Default for SignalMask {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-process signal state
pub struct SignalState {
    // Signal handler for each signal (1-32)
    handlers: [SigAction; 32],
    // Pending signals to be delivered
    pending: SignalMask,
    // Currently blocked signals
    blocked: SignalMask,
}

impl SignalState {
    pub fn new() -> Self {
        let mut handlers = [SigAction::default(); 32];

        // Initialize handlers with default actions
        for i in 0..32 {
            if let Some(sig) = Signal::try_from_u32(i as u32 + 1) {
                handlers[i].handler = get_default_action(sig);
            }
        }

        Self {
            handlers,
            pending: SignalMask::new(),
            blocked: SignalMask::new(),
        }
    }

    pub fn set_handler(&mut self, signal: Signal, action: SigAction) -> Result<(), SignalError> {
        // SIGKILL and SIGSTOP cannot be caught
        match signal {
            Signal::SIGKILL | Signal::SIGSTOP => {
                if matches!(action.handler, SigHandler::UserHandler(_)) {
                    return Err(SignalError::CannotCatch);
                }
            }
            _ => {}
        }

        let idx = (signal.as_u32() - 1) as usize;
        if idx >= 32 {
            return Err(SignalError::InvalidSignal);
        }

        self.handlers[idx] = action;
        Ok(())
    }

    pub fn get_handler(&self, signal: Signal) -> SigAction {
        let idx = (signal.as_u32() - 1) as usize;
        if idx >= 32 {
            return SigAction::default();
        }
        self.handlers[idx]
    }

    pub fn deliver_signal(&mut self, signal: Signal) {
        self.pending.add(signal);
    }

    pub fn is_pending(&self, signal: Signal) -> bool {
        self.pending.contains(signal)
    }

    pub fn clear_pending(&mut self, signal: Signal) {
        self.pending.remove(signal);
    }

    pub fn pending_signals(&self) -> SignalMask {
        self.pending
    }

    pub fn set_blocked_mask(&mut self, mask: u64) {
        self.blocked.set_bits(mask);
    }

    pub fn get_blocked_mask(&self) -> u64 {
        self.blocked.bits()
    }

    pub fn is_blocked(&self, signal: Signal) -> bool {
        self.blocked.contains(signal)
    }
}

impl Default for SignalState {
    fn default() -> Self {
        Self::new()
    }
}
