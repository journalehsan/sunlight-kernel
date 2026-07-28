//! SunlightOS Welcome Wizard — pure onboarding model and greeting providers.
//!
//! Phase 1: local deterministic greeting with a future Wise Owl provider hook.
//! Host-testable without display server or Wise Owl backend.

#![cfg_attr(not(test), no_std)]

use core::fmt::Write;
use heapless::String;

/// Trusted bundle identity for session catalog and completion IPC.
/// Kept short (≤24) to avoid accidental heapless String capacity mismatches.
pub const BUNDLE_ID: &str = "org.sunlight.welcome";
/// Legacy id from early Phase 1 builds (migrated by sessiond).
pub const LEGACY_BUNDLE_ID: &str = "org.sunlight.wiseowl-welcome";
pub const DISPLAY_NAME: &str = "Welcome to SunlightOS";
pub const APP_VERSION: &str = "0.1.0";

pub const MAX_GREETING: usize = 240;
pub const MAX_NAME: usize = 48;
pub const MAX_LOCALE: usize = 16;
pub const MAX_VERSION: usize = 32;
pub const MAX_MODEL: usize = 48;

// ── Launch mode ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    /// Launched by sessiond after Shell Ready (auto onboarding).
    Automatic,
    /// Launched from the app launcher or CLI (`--manual`).
    Manual,
}

impl LaunchMode {
    pub fn from_args(args: &[&str]) -> Self {
        for a in args {
            if *a == "--manual" || *a == "--mode=manual" {
                return LaunchMode::Manual;
            }
        }
        LaunchMode::Automatic
    }
}

// ── Greeting request / response ──────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineSummary {
    pub cpu_cores: Option<u32>,
    /// Total RAM in MiB when known (rounded).
    pub ram_mib: Option<u64>,
    pub device_class: Option<String<16>>,
    pub model_name: Option<String<MAX_MODEL>>,
    pub screen_w: Option<u32>,
    pub screen_h: Option<u32>,
}

impl MachineSummary {
    pub fn empty() -> Self {
        Self {
            cpu_cores: None,
            ram_mib: None,
            device_class: None,
            model_name: None,
            screen_w: None,
            screen_h: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WelcomeGreetingRequest {
    pub user_display_name: Option<String<MAX_NAME>>,
    pub locale: Option<String<MAX_LOCALE>>,
    pub sunlight_version: String<MAX_VERSION>,
    pub machine_summary: MachineSummary,
    pub first_login: bool,
    pub first_after_upgrade: bool,
}

impl WelcomeGreetingRequest {
    pub fn validate(&self) -> Result<(), GreetingError> {
        if self.sunlight_version.is_empty() {
            return Err(GreetingError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WelcomeGreeting {
    pub text: String<MAX_GREETING>,
    pub source: GreetingSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingSource {
    LocalFallback,
    WiseOwl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingError {
    InvalidRequest,
    Unavailable,
    Timeout,
    ProviderError,
}

pub trait WelcomeGreetingProvider {
    fn generate_greeting(
        &self,
        request: &WelcomeGreetingRequest,
    ) -> Result<WelcomeGreeting, GreetingError>;
}

// ── Local fallback provider ──────────────────────────────────────────────────

pub struct LocalGreetingProvider;

impl WelcomeGreetingProvider for LocalGreetingProvider {
    fn generate_greeting(
        &self,
        request: &WelcomeGreetingRequest,
    ) -> Result<WelcomeGreeting, GreetingError> {
        request.validate()?;
        let mut text = String::<MAX_GREETING>::new();
        let _ = text.push_str("Welcome to SunlightOS");
        if let Some(name) = request.user_display_name.as_ref() {
            if !name.is_empty() {
                let _ = write!(&mut text, ", {}", name.as_str());
            }
        }
        let _ = text.push_str(". Your desktop is ready.");
        if let Some(cores) = request.machine_summary.cpu_cores {
            if let Some(ram) = request.machine_summary.ram_mib {
                let _ = write!(
                    &mut text,
                    " This system has {} CPU cores and about {} MiB of memory.",
                    cores, ram
                );
            } else {
                let _ = write!(&mut text, " This system has {} CPU cores.", cores);
            }
        } else if let Some(ram) = request.machine_summary.ram_mib {
            let _ = write!(&mut text, " This system has about {} MiB of memory.", ram);
        }
        if request.first_after_upgrade {
            let _ = text.push_str(" Thanks for updating — here is a quick tour of what is new.");
        } else if request.first_login {
            let _ = text.push_str(" Take a short tour to learn the basics.");
        } else {
            let _ = text.push_str(" Browse the tour anytime, or jump to a next step below.");
        }
        Ok(WelcomeGreeting {
            text,
            source: GreetingSource::LocalFallback,
        })
    }
}

// ── Future Wise Owl provider (stub) ──────────────────────────────────────────

/// Capability-checked stub. Always returns Unavailable in Phase 1.
///
/// Future work: query Wise Owl when present, with a short timeout, then fall back.
pub struct FutureWiseOwlGreetingProvider {
    pub available: bool,
}

impl FutureWiseOwlGreetingProvider {
    pub const fn stub() -> Self {
        Self { available: false }
    }
}

impl WelcomeGreetingProvider for FutureWiseOwlGreetingProvider {
    fn generate_greeting(
        &self,
        request: &WelcomeGreetingRequest,
    ) -> Result<WelcomeGreeting, GreetingError> {
        request.validate()?;
        if !self.available {
            return Err(GreetingError::Unavailable);
        }
        // No Wise Owl backend in this phase.
        Err(GreetingError::Unavailable)
    }
}

/// Prefer Wise Owl when available; otherwise local fallback. Never blocks.
pub fn resolve_greeting(
    request: &WelcomeGreetingRequest,
    wise_owl: &FutureWiseOwlGreetingProvider,
    local: &LocalGreetingProvider,
) -> WelcomeGreeting {
    match wise_owl.generate_greeting(request) {
        Ok(g) => g,
        Err(_) => local
            .generate_greeting(request)
            .unwrap_or_else(|_| WelcomeGreeting {
                text: {
                    let mut t = String::new();
                    let _ = t.push_str("Welcome to SunlightOS. Your desktop is ready.");
                    t
                },
                source: GreetingSource::LocalFallback,
            }),
    }
}

// ── Wizard pages / flow ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardPage {
    ImmediateWelcome,
    Greeting,
    Slide(usize),
    Actions,
}

pub const SLIDE_COUNT: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideContent {
    pub title: &'static str,
    pub body: &'static str,
}

/// Static feature introduction slides (deterministic, localizable string table).
pub const SLIDES: [SlideContent; SLIDE_COUNT] = [
    SlideContent {
        title: "Desktop and panels",
        body: "Vortex Shell gives you a clean desktop, dock, Start menu, and status area. Windows stay organized and responsive.",
    },
    SlideContent {
        title: "Search and launching apps",
        body: "Open apps from the Start menu or search palette. Pin favorites to the dock for one-click access.",
    },
    SlideContent {
        title: "Control Panel",
        body: "Personalize wallpaper, display, mouse, notifications, and more in System Preferences.",
    },
    SlideContent {
        title: "Files and Documents",
        body: "Browse your home folder, Documents, and drives with Sunlight Files — local-first and straightforward.",
    },
    SlideContent {
        title: "Terminal and developer tools",
        body: "Sunlight Terminal and Chronos DOS tools are ready when you need a command line or classic apps.",
    },
    SlideContent {
        title: "Reliability and updates",
        body: "SunlightOS aims for calm, atomic system updates so your desktop stays dependable over time.",
    },
    SlideContent {
        title: "Local-first privacy and Wise Owl",
        body: "Your data stays on your machine. Wise Owl will later offer helpful local assistance — never required for the desktop.",
    },
];

// ── Action cards ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Open an existing application via sun-exec alias.
    OpenApp { command: &'static str },
    /// Open Control Panel at a page.
    OpenControlPanel { page: &'static str },
    /// Honest placeholder — no side effect.
    ComingSoon,
    /// Show in-app about text.
    AboutWelcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionCard {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub kind: ActionKind,
    pub placeholder_honest: bool,
}

pub const ACTION_CARDS: [ActionCard; 6] = [
    ActionCard {
        id: "personalize",
        title: "Personalize my desktop",
        description: "Open wallpaper settings in Control Panel.",
        kind: ActionKind::OpenControlPanel { page: "wallpaper" },
        placeholder_honest: false,
    },
    ActionCard {
        id: "control-panel",
        title: "Open Control Panel",
        description: "Browse system preferences.",
        kind: ActionKind::OpenApp {
            command: "settings",
        },
        placeholder_honest: false,
    },
    ActionCard {
        id: "files",
        title: "Browse my files",
        description: "Open Sunlight Files.",
        kind: ActionKind::OpenApp { command: "files" },
        placeholder_honest: false,
    },
    ActionCard {
        id: "terminal",
        title: "Open Terminal",
        description: "Launch Sunlight Terminal.",
        kind: ActionKind::OpenApp {
            command: "terminal",
        },
        placeholder_honest: false,
    },
    ActionCard {
        id: "about-os",
        title: "Learn more about SunlightOS",
        description: "Open About SunlightOS.",
        kind: ActionKind::OpenControlPanel { page: "about-os" },
        placeholder_honest: false,
    },
    ActionCard {
        id: "wise-owl",
        title: "Meet Wise Owl later",
        description: "Coming soon when Wise Owl gains interactive system actions.",
        kind: ActionKind::ComingSoon,
        placeholder_honest: true,
    },
];

// ── Wizard model ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// User finished the tour; record session onboarding completion (auto mode).
    Finished,
    /// User skipped/dismissed early; do not complete one-time policy.
    DismissedIncomplete,
    /// Manual Welcome Center closed without changing policy.
    ManualClosed,
}

#[derive(Clone, Debug)]
pub struct WizardState {
    pub mode: LaunchMode,
    pub page: WizardPage,
    pub greeting: Option<WelcomeGreeting>,
    pub greeting_requested: bool,
    pub finished: bool,
    pub onboarding_already_complete: bool,
    pub first_login: bool,
    pub first_after_upgrade: bool,
    pub selected_action: Option<usize>,
    pub action_status: String<96>,
    pub sunlight_version: String<MAX_VERSION>,
    pub machine: MachineSummary,
}

impl WizardState {
    pub fn new(mode: LaunchMode) -> Self {
        let mut sunlight_version = String::new();
        let _ = sunlight_version.push_str(env!("CARGO_PKG_VERSION"));
        Self {
            mode,
            page: WizardPage::ImmediateWelcome,
            greeting: None,
            greeting_requested: false,
            finished: false,
            onboarding_already_complete: false,
            first_login: mode == LaunchMode::Automatic,
            first_after_upgrade: mode == LaunchMode::Automatic,
            selected_action: None,
            action_status: String::new(),
            sunlight_version,
            machine: MachineSummary::empty(),
        }
    }

    pub fn enter_welcome_center(&mut self) {
        self.onboarding_already_complete = true;
        self.page = WizardPage::ImmediateWelcome;
        self.first_login = false;
        self.first_after_upgrade = false;
    }

    pub fn begin(&mut self) {
        self.page = WizardPage::Greeting;
        self.greeting_requested = true;
    }

    pub fn ensure_greeting(&mut self) {
        if self.greeting.is_some() {
            return;
        }
        let mut version = String::new();
        let _ = version.push_str(self.sunlight_version.as_str());
        let request = WelcomeGreetingRequest {
            user_display_name: None,
            locale: None,
            sunlight_version: version,
            machine_summary: self.machine.clone(),
            first_login: self.first_login,
            first_after_upgrade: self.first_after_upgrade,
        };
        let greeting = resolve_greeting(
            &request,
            &FutureWiseOwlGreetingProvider::stub(),
            &LocalGreetingProvider,
        );
        self.greeting = Some(greeting);
        self.greeting_requested = true;
    }

    pub fn continue_from_greeting(&mut self) {
        self.page = WizardPage::Slide(0);
    }

    pub fn next_slide(&mut self) {
        match self.page {
            WizardPage::Slide(i) if i + 1 < SLIDE_COUNT => {
                self.page = WizardPage::Slide(i + 1);
            }
            WizardPage::Slide(_) => {
                self.page = WizardPage::Actions;
            }
            _ => {}
        }
    }

    pub fn prev_slide(&mut self) {
        match self.page {
            WizardPage::Slide(0) => {
                self.page = WizardPage::Greeting;
            }
            WizardPage::Slide(i) => {
                self.page = WizardPage::Slide(i - 1);
            }
            WizardPage::Actions => {
                self.page = WizardPage::Slide(SLIDE_COUNT - 1);
            }
            WizardPage::Greeting => {
                self.page = WizardPage::ImmediateWelcome;
            }
            _ => {}
        }
    }

    pub fn skip_to_actions(&mut self) {
        self.page = WizardPage::Actions;
    }

    pub fn finish(&mut self) -> CompletionOutcome {
        self.finished = true;
        match self.mode {
            LaunchMode::Automatic => CompletionOutcome::Finished,
            LaunchMode::Manual => {
                // Manual finish does not re-arm or re-consume upgrade policy unless
                // this was an explicit upgrade tour completion (automatic mode only).
                CompletionOutcome::Finished
            }
        }
    }

    pub fn dismiss_early(&mut self) -> CompletionOutcome {
        self.finished = true;
        match self.mode {
            LaunchMode::Automatic => CompletionOutcome::DismissedIncomplete,
            LaunchMode::Manual => CompletionOutcome::ManualClosed,
        }
    }

    pub fn current_slide(&self) -> Option<&'static SlideContent> {
        match self.page {
            WizardPage::Slide(i) => SLIDES.get(i),
            _ => None,
        }
    }

    pub fn set_action_status(&mut self, msg: &str) {
        self.action_status.clear();
        let _ = self.action_status.push_str(msg);
    }
}

/// Whether the wizard should call `SESSION_STARTUP_COMPLETE`.
///
/// Only an explicit **Finished** outcome reports completion. Early dismiss and
/// manual-center close never consume one-time policy. Sessiond still
/// capability-checks the caller process.
pub fn should_mark_onboarding_complete(outcome: CompletionOutcome) -> bool {
    matches!(outcome, CompletionOutcome::Finished)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_greeting_works() {
        let mut version = String::new();
        version.push_str("0.2.0").unwrap();
        let mut machine = MachineSummary::empty();
        machine.cpu_cores = Some(4);
        machine.ram_mib = Some(2048);
        let req = WelcomeGreetingRequest {
            user_display_name: None,
            locale: None,
            sunlight_version: version,
            machine_summary: machine,
            first_login: true,
            first_after_upgrade: false,
        };
        let g = LocalGreetingProvider.generate_greeting(&req).unwrap();
        assert!(g.text.as_str().contains("Welcome to SunlightOS"));
        assert!(g.text.as_str().contains("4 CPU cores"));
        assert!(g.text.as_str().contains("2048"));
        assert_eq!(g.source, GreetingSource::LocalFallback);
    }

    #[test]
    fn invalid_request_rejected() {
        let req = WelcomeGreetingRequest {
            user_display_name: None,
            locale: None,
            sunlight_version: String::new(),
            machine_summary: MachineSummary::empty(),
            first_login: false,
            first_after_upgrade: false,
        };
        assert_eq!(
            LocalGreetingProvider.generate_greeting(&req),
            Err(GreetingError::InvalidRequest)
        );
    }

    #[test]
    fn wise_owl_unavailable_falls_back() {
        let mut version = String::new();
        version.push_str("1.0.0").unwrap();
        let req = WelcomeGreetingRequest {
            user_display_name: None,
            locale: None,
            sunlight_version: version,
            machine_summary: MachineSummary::empty(),
            first_login: true,
            first_after_upgrade: false,
        };
        let g = resolve_greeting(
            &req,
            &FutureWiseOwlGreetingProvider::stub(),
            &LocalGreetingProvider,
        );
        assert_eq!(g.source, GreetingSource::LocalFallback);
        assert!(!g.text.is_empty());
    }

    #[test]
    fn page_flow_order() {
        let mut w = WizardState::new(LaunchMode::Automatic);
        assert_eq!(w.page, WizardPage::ImmediateWelcome);
        w.begin();
        assert_eq!(w.page, WizardPage::Greeting);
        w.ensure_greeting();
        assert!(w.greeting.is_some());
        w.continue_from_greeting();
        assert_eq!(w.page, WizardPage::Slide(0));
        for _ in 0..SLIDE_COUNT {
            w.next_slide();
        }
        assert_eq!(w.page, WizardPage::Actions);
        w.prev_slide();
        assert_eq!(w.page, WizardPage::Slide(SLIDE_COUNT - 1));
    }

    #[test]
    fn skip_goes_to_actions() {
        let mut w = WizardState::new(LaunchMode::Automatic);
        w.skip_to_actions();
        assert_eq!(w.page, WizardPage::Actions);
    }

    #[test]
    fn finish_marks_complete_not_launch() {
        let mut w = WizardState::new(LaunchMode::Automatic);
        // Merely constructed / launched does not finish.
        assert!(!w.finished);
        assert!(!should_mark_onboarding_complete(
            CompletionOutcome::DismissedIncomplete
        ));
        let outcome = w.finish();
        assert_eq!(outcome, CompletionOutcome::Finished);
        assert!(should_mark_onboarding_complete(outcome));
    }

    #[test]
    fn early_dismiss_does_not_complete() {
        let mut w = WizardState::new(LaunchMode::Automatic);
        let outcome = w.dismiss_early();
        assert_eq!(outcome, CompletionOutcome::DismissedIncomplete);
        assert!(!should_mark_onboarding_complete(outcome));
    }

    #[test]
    fn manual_mode_distinguished() {
        assert_eq!(
            LaunchMode::from_args(&["welcome", "--manual"]),
            LaunchMode::Manual
        );
        assert_eq!(LaunchMode::from_args(&["welcome"]), LaunchMode::Automatic);
        let mut w = WizardState::new(LaunchMode::Manual);
        w.enter_welcome_center();
        assert!(w.onboarding_already_complete);
        assert_eq!(w.dismiss_early(), CompletionOutcome::ManualClosed);
    }

    #[test]
    fn action_cards_placeholders_honest() {
        for card in ACTION_CARDS.iter() {
            if matches!(card.kind, ActionKind::ComingSoon) {
                assert!(card.placeholder_honest);
                assert!(
                    card.description.contains("Coming soon")
                        || card.description.contains("available when")
                );
            }
        }
    }

    #[test]
    fn slide_count_bounded() {
        assert!(SLIDE_COUNT <= 12);
        assert!(!SLIDES.is_empty());
    }
}
