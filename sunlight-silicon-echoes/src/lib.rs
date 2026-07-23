//! Story data, validation, director boundary, and save format for Silicon Echoes: 1993.
//!
//! The graph is deterministic. Ambient presentation may vary, but story state,
//! consequences, and scene order are driven only by this module.

#![no_std]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const SAVE_FORMAT_VERSION: u16 = 4;
pub const MAX_SAVE_BYTES: usize = 4096;
const MAX_RECORDS: usize = 192;
const MAX_TEXT_VALUE_BYTES: usize = 128;
const MAX_VISITED_NODES: usize = 64;
const MAX_STATE_SET_ITEMS: usize = 48;
const MAX_RELATIONSHIPS: usize = 8;
const MAX_TENDENCIES: usize = 8;
const MAX_SELECTED_CHOICES: usize = 16;
pub const START_NODE: StoryNodeId = StoryNodeId("bedroom.wake");
pub const TEMPORARY_ENDING: EndingId = EndingId("ending.chapter-one");
pub const CHAPTER_TWO_ENDING: EndingId = EndingId("ending.chapter-two");

/// The non-canonical presentation lifecycle for a narrative scene.  This is
/// deliberately separate from [`WorldState`]: a save restores a stable story
/// node and starts its presentation again instead of serialising animation
/// progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePresentation {
    Entering,
    Revealing,
    PostRevealPause,
    AwaitingChoice,
    Transitioning,
}

/// Shared, bounded timing knobs for narrative presentation.  They are kept in
/// one place so an eventual accessibility settings screen can select a speed
/// profile without changing scene data or story outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationConfig {
    pub entrance_pause_ms: u64,
    pub grapheme_delay_ms: u64,
    pub clause_pause_ms: u64,
    pub sentence_pause_ms: u64,
    pub paragraph_pause_ms: u64,
    pub post_reveal_pause_ms: u64,
    pub instant_text: bool,
}

/// Bounded presentation profiles.  [`PresentationProfile::Normal`] is the
/// default readable pacing; [`PresentationProfile::Instant`] is for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationProfile {
    Slow,
    Normal,
    Fast,
    Instant,
}

impl Default for PresentationConfig {
    fn default() -> Self {
        PresentationProfile::Normal.config()
    }
}

impl PresentationProfile {
    pub fn config(self) -> PresentationConfig {
        match self {
            PresentationProfile::Slow => PresentationConfig {
                entrance_pause_ms: 500,
                grapheme_delay_ms: 55,
                clause_pause_ms: 180,
                sentence_pause_ms: 380,
                paragraph_pause_ms: 500,
                post_reveal_pause_ms: 650,
                instant_text: false,
            },
            PresentationProfile::Normal => PresentationConfig {
                entrance_pause_ms: 420,
                grapheme_delay_ms: 50,
                clause_pause_ms: 150,
                sentence_pause_ms: 320,
                paragraph_pause_ms: 420,
                post_reveal_pause_ms: 520,
                instant_text: false,
            },
            PresentationProfile::Fast => PresentationConfig {
                entrance_pause_ms: 220,
                grapheme_delay_ms: 28,
                clause_pause_ms: 80,
                sentence_pause_ms: 160,
                paragraph_pause_ms: 240,
                post_reveal_pause_ms: 280,
                instant_text: false,
            },
            PresentationProfile::Instant => PresentationConfig {
                entrance_pause_ms: 0,
                grapheme_delay_ms: 0,
                clause_pause_ms: 0,
                sentence_pause_ms: 0,
                paragraph_pause_ms: 0,
                post_reveal_pause_ms: 0,
                instant_text: true,
            },
        }
    }
}

/// Maximum graphemes advanced in a single [`NarrativePresentation::tick`].
/// Catches up after a delayed frame without stalling the renderer on long
/// punctuation-free runs.
pub const MAX_REVEALS_PER_TICK: usize = 48;

/// Previous Normal defaults (pre typography-polish).  Kept for regression tests.
pub const LEGACY_NORMAL_GRAPHEME_DELAY_MS: u64 = 32;
pub const LEGACY_NORMAL_SENTENCE_PAUSE_MS: u64 = 180;

/// A small timeline driven by the caller's monotonic clock.  `boundaries`
/// contains Unicode scalar boundaries (the strongest portable boundary in this
/// no_std crate); rendering can therefore always slice valid UTF-8 without
/// allocating a growing prefix on every frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NarrativePresentation {
    text: String,
    boundaries: Vec<usize>,
    revealed: usize,
    state: ScenePresentation,
    deadline_ms: u64,
    config: PresentationConfig,
}

impl NarrativePresentation {
    pub fn new(text: String, now_ms: u64, config: PresentationConfig) -> Self {
        let boundaries: Vec<usize> = text
            .char_indices()
            .map(|(index, ch)| index + ch.len_utf8())
            .collect();
        let instant = config.instant_text;
        let revealed = if instant { boundaries.len() } else { 0 };
        let state = if instant {
            ScenePresentation::AwaitingChoice
        } else {
            ScenePresentation::Entering
        };
        let deadline_ms = if instant {
            now_ms
        } else {
            now_ms.saturating_add(config.entrance_pause_ms)
        };
        Self {
            text,
            boundaries,
            revealed,
            state,
            deadline_ms,
            config,
        }
    }

    pub fn state(&self) -> ScenePresentation {
        self.state
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn visible_byte_end(&self) -> usize {
        if self.revealed == 0 {
            return 0;
        }
        self.boundaries.get(self.revealed - 1).copied().unwrap_or(0)
    }

    pub fn revealed_count(&self) -> usize {
        self.revealed
    }

    pub fn boundary_count(&self) -> usize {
        self.boundaries.len()
    }

    pub fn choices_visible(&self) -> bool {
        self.state == ScenePresentation::AwaitingChoice
    }

    pub fn is_revealing(&self) -> bool {
        self.state == ScenePresentation::Revealing
    }

    pub fn begin_transition(&mut self) {
        self.state = ScenePresentation::Transitioning;
    }

    /// Advance through any elapsed deadlines.  Catching up happens in this one
    /// bounded loop, rather than by posting one timer event per character.
    ///
    /// When a frame is delayed, every elapsed deadline is honoured so
    /// punctuation pauses are never skipped, but the number of graphemes
    /// revealed in a single call is capped to keep rendering responsive.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.config.instant_text || self.state == ScenePresentation::Transitioning {
            return false;
        }
        let before = (self.state, self.revealed, self.deadline_ms);
        let mut reveals = 0usize;
        loop {
            if now_ms < self.deadline_ms {
                break;
            }
            match self.state {
                ScenePresentation::Entering => {
                    self.state = ScenePresentation::Revealing;
                    self.deadline_ms = self
                        .deadline_ms
                        .saturating_add(self.config.grapheme_delay_ms);
                }
                ScenePresentation::Revealing => {
                    if reveals >= MAX_REVEALS_PER_TICK {
                        break;
                    }
                    if self.revealed < self.boundaries.len() {
                        self.revealed += 1;
                        reveals += 1;
                        self.deadline_ms = self.deadline_ms.saturating_add(self.reveal_delay());
                    }
                    if self.revealed == self.boundaries.len() {
                        self.state = ScenePresentation::PostRevealPause;
                        self.deadline_ms = now_ms.saturating_add(self.config.post_reveal_pause_ms);
                    }
                }
                ScenePresentation::PostRevealPause => {
                    self.state = ScenePresentation::AwaitingChoice;
                    break;
                }
                ScenePresentation::AwaitingChoice | ScenePresentation::Transitioning => break,
            }
        }
        before != (self.state, self.revealed, self.deadline_ms)
    }

    /// Finish prose only.  The following post-reveal pause deliberately keeps
    /// the same Enter/Space press from becoming a choice activation.
    pub fn skip_reveal(&mut self, now_ms: u64) -> bool {
        if self.state != ScenePresentation::Revealing {
            return false;
        }
        self.revealed = self.boundaries.len();
        self.state = ScenePresentation::PostRevealPause;
        self.deadline_ms = now_ms.saturating_add(self.config.post_reveal_pause_ms);
        true
    }

    /// Delay applied after the most recently revealed scalar.  Spaces keep the
    /// ordinary base delay (no extra pause); punctuation and paragraph breaks
    /// lengthen the rhythm so the final sentence can land before choices.
    pub fn reveal_delay_after_visible(&self) -> u64 {
        self.reveal_delay()
    }

    fn reveal_delay(&self) -> u64 {
        let end = self.visible_byte_end();
        if end == 0 {
            return self.config.grapheme_delay_ms;
        }
        let prefix = &self.text[..end];
        if prefix.ends_with("\n\n") {
            return self.config.paragraph_pause_ms;
        }
        match prefix.chars().last() {
            Some('.') | Some('!') | Some('?') | Some('\u{2026}') => self.config.sentence_pause_ms,
            Some(',') | Some(';') | Some(':') | Some('\u{2014}') | Some('\u{2013}') => {
                self.config.clause_pause_ms
            }
            // Spaces and ordinary glyphs share the base delay so word gaps do
            // not stretch the typewriter into a sluggish staccato.
            Some(ch) if ch.is_whitespace() => self.config.grapheme_delay_ms,
            _ => self.config.grapheme_delay_ms,
        }
    }
}

/// Guards printable shortcuts when the UI only receives decoded character
/// events.  A shortcut observed before choices exist remains blocked until the
/// key stream has gone quiet, so a held/repeated key cannot leak into the next
/// scene.  It is presentation-only state and is never saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShortcutGate {
    blocked_letters: u32,
    last_letter_ms: u64,
}

impl ShortcutGate {
    pub const QUIET_RELEASE_MS: u64 = 180;

    pub fn shortcut_index(
        &mut self,
        ch: char,
        choices_are_active: bool,
        now_ms: u64,
    ) -> Option<usize> {
        let letter = ch.to_ascii_uppercase();
        if !letter.is_ascii_uppercase() {
            return None;
        }
        let index = (letter as u8 - b'A') as usize;
        let bit = 1u32.checked_shl(index as u32)?;
        if !choices_are_active {
            self.blocked_letters |= bit;
            self.last_letter_ms = now_ms;
            return None;
        }
        if now_ms.saturating_sub(self.last_letter_ms) >= Self::QUIET_RELEASE_MS {
            self.blocked_letters = 0;
        }
        self.last_letter_ms = now_ms;
        if self.blocked_letters & bit != 0 {
            return None;
        }
        self.blocked_letters |= bit;
        Some(index)
    }

    pub fn clear(&mut self) {
        self.blocked_letters = 0;
        self.last_letter_ms = 0;
    }

    pub fn block_until_quiet(&mut self, now_ms: u64) {
        self.blocked_letters = u32::MAX;
        self.last_letter_ms = now_ms;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SceneId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoryNodeId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChoiceId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndingId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoLayer {
    Physical1993,
    Revision2013,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotId {
    Clock,
    Workstation,
    Desk,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tendency {
    Agency,
    Responsibility,
    Curiosity,
    Attachment,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoryFlags {
    values: BTreeMap<String, bool>,
}

impl StoryFlags {
    pub fn get(&self, key: &str) -> bool {
        self.values.get(key).copied().unwrap_or(false)
    }

    pub fn set(&mut self, key: &str, value: bool) {
        self.values.insert(String::from(key), value);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &bool)> {
        self.values.iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelayedConsequence {
    pub id: &'static str,
    pub after_node: StoryNodeId,
    pub effect: DelayedEffect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelayedEffect {
    SetFlag(&'static str, bool),
    AddObservation(&'static str),
    AdjustRelationship(ActorId, i8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Consequence {
    SetFlag(&'static str, bool),
    Shift(Tendency, i8),
    AddFact(&'static str),
    AddObservation(&'static str),
    AddBelief(&'static str),
    RemoveBelief(&'static str),
    AddActorKnowledge(&'static str),
    AddActorBelief(&'static str),
    Remember(&'static str),
    AdjustRelationship(ActorId, i8),
    QueueDelayed {
        id: &'static str,
        after_node: StoryNodeId,
        effect: DelayedEffect,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Flag(&'static str, bool),
    Fact(&'static str),
    Observation(&'static str),
    Belief(&'static str),
    EchoLayer(EchoLayer),
    Visited(StoryNodeId),
    All(&'static [Condition]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Node(StoryNodeId),
    Ending(EndingId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoryAction {
    pub id: ChoiceId,
    pub target: Transition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectorDecision {
    pub action: Option<ChoiceId>,
    pub target: Option<Transition>,
}

pub trait Director {
    fn next(&mut self, world: &WorldState, available: &[StoryAction]) -> DirectorDecision;
}

pub struct ScriptedDirector {
    selected: ChoiceId,
}

impl ScriptedDirector {
    pub fn choose(selected: ChoiceId) -> Self {
        Self { selected }
    }
}

impl Director for ScriptedDirector {
    fn next(&mut self, _: &WorldState, available: &[StoryAction]) -> DirectorDecision {
        let selected = available.iter().find(|action| action.id == self.selected);
        DirectorDecision {
            action: selected.map(|action| action.id),
            target: selected.map(|action| action.target),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub id: ChoiceId,
    pub text: &'static str,
    pub target: Transition,
    pub condition: Option<Condition>,
    pub effects: &'static [Consequence],
    pub intentionally_converges: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct StoryNode {
    pub id: StoryNodeId,
    pub scene: SceneId,
    pub narration: &'static str,
    pub choices: &'static [Choice],
    pub entry_effects: &'static [Consequence],
    pub uncontrolled_event: bool,
    pub automatic_target: Option<Transition>,
}

#[derive(Clone, Copy, Debug)]
pub struct Hotspot {
    pub id: HotspotId,
    pub label: &'static str,
    pub target: StoryNodeId,
    pub condition: Option<Condition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneObjectKind {
    Structural,
    Decorative,
    Interactive,
    Actor,
    Stateful,
}

#[derive(Clone, Copy, Debug)]
pub struct SceneObject {
    pub id: ObjectId,
    pub kind: SceneObjectKind,
    pub label: &'static str,
    pub action: Option<ChoiceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoObjectLayer {
    Both,
    Physical1993,
    Revision2013,
}

#[derive(Clone, Copy, Debug)]
pub struct EchoObject {
    pub id: ObjectId,
    pub layer: EchoObjectLayer,
    pub label: &'static str,
    pub action: Option<ChoiceId>,
}

#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub id: SceneId,
    pub title: &'static str,
    pub hotspots: &'static [Hotspot],
    pub objects: &'static [SceneObject],
}

const RILEY: ActorId = ActorId("riley");
const VALE: ActorId = ActorId("vale");
const LIO: ActorId = ActorId("lio");
const ELIAS: ActorId = ActorId("elias");
const NO_EFFECTS: &[Consequence] = &[];
const NO_CHOICES: &[Choice] = &[];

const CLOCK_CHOICES: &[Choice] = &[Choice {
    id: ChoiceId("clock.accept-date"),
    text: "Let the date remain impossible.",
    target: Transition::Node(StoryNodeId("bedroom.after-clock")),
    condition: None,
    effects: &[
        Consequence::SetFlag("saw_date", true),
        Consequence::AddObservation("clock_1993"),
        Consequence::Shift(Tendency::Responsibility, 1),
    ],
    intentionally_converges: false,
}];

const WORKSTATION_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("workstation.read-prompt"),
        text: "Read the waiting prompt.",
        target: Transition::Node(StoryNodeId("bedroom.after-workstation")),
        condition: None,
        effects: &[
            Consequence::SetFlag("saw_prompt", true),
            Consequence::AddObservation("waiting_prompt"),
            Consequence::Shift(Tendency::Curiosity, 1),
            Consequence::QueueDelayed {
                id: "signal-after-window",
                after_node: StoryNodeId("bedroom.window"),
                effect: DelayedEffect::SetFlag("signal_arrived", true),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("workstation.turn-away"),
        text: "Turn away from the green cursor.",
        target: Transition::Node(StoryNodeId("bedroom.after-workstation")),
        condition: None,
        effects: &[Consequence::Shift(Tendency::Attachment, 1)],
        intentionally_converges: true,
    },
];

const DESK_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("desk.open-letter"),
        text: "Open the letter addressed in your handwriting.",
        target: Transition::Node(StoryNodeId("bedroom.after-desk")),
        condition: None,
        effects: &[
            Consequence::SetFlag("opened_letter", true),
            Consequence::AddObservation("letter_in_handwriting"),
            Consequence::Shift(Tendency::Attachment, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("desk.leave-letter"),
        text: "Leave the letter sealed.",
        target: Transition::Node(StoryNodeId("bedroom.after-desk")),
        condition: None,
        effects: &[Consequence::Shift(Tendency::Agency, 1)],
        intentionally_converges: true,
    },
];

const WINDOW_CHOICES: &[Choice] = &[Choice {
    id: ChoiceId("window.answer-signal"),
    text: "Answer the signal.",
    target: Transition::Node(StoryNodeId("bedroom.signal")),
    condition: Some(Condition::Flag("saw_date", true)),
    effects: &[Consequence::Shift(Tendency::Agency, 1)],
    intentionally_converges: false,
}];

const SIGNAL_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("signal.claim-self"),
        text: "Say: I am still here.",
        target: Transition::Node(StoryNodeId("chapter.hallway")),
        condition: None,
        effects: &[
            Consequence::AddBelief("signal_is_mara"),
            Consequence::Remember("signal_claimed"),
            Consequence::Shift(Tendency::Responsibility, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("signal-listen"),
        text: "Say nothing. Listen.",
        target: Transition::Node(StoryNodeId("chapter.hallway")),
        condition: None,
        effects: &[
            Consequence::AddBelief("signal_is_recording"),
            Consequence::Remember("signal_listened"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
];

const HALLWAY_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("hallway.inspect-note"),
        text: "Read the note taped beside your door.",
        target: Transition::Node(StoryNodeId("chapter.kitchen")),
        condition: None,
        effects: &[
            Consequence::AddObservation("wake_note"),
            Consequence::AddFact("someone_expected_mara"),
            Consequence::QueueDelayed {
                id: "riley-waited",
                after_node: StoryNodeId("chapter.diner"),
                effect: DelayedEffect::AddObservation("riley_waited"),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("hallway.leave-note"),
        text: "Leave the note where someone meant to find it.",
        target: Transition::Node(StoryNodeId("chapter.kitchen")),
        condition: None,
        effects: &[Consequence::Remember("left_wake_note")],
        intentionally_converges: true,
    },
];

const KITCHEN_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("kitchen.read-newspaper"),
        text: "Read the newspaper under the cold coffee.",
        target: Transition::Node(StoryNodeId("chapter.landing")),
        condition: None,
        effects: &[
            Consequence::AddFact("year_is_1993"),
            Consequence::AddObservation("newspaper_1993"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("kitchen.study-photo"),
        text: "Study the family photo with tomorrow's date on its back.",
        target: Transition::Node(StoryNodeId("chapter.landing")),
        condition: None,
        effects: &[
            Consequence::AddObservation("future_dated_photo"),
            Consequence::AddBelief("return_was_planned"),
            Consequence::Shift(Tendency::Attachment, 1),
        ],
        intentionally_converges: true,
    },
];

const LANDING_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("landing.take-card"),
        text: "Keep the archive appointment card.",
        target: Transition::Node(StoryNodeId("chapter.stairwell")),
        condition: None,
        effects: &[
            Consequence::AddObservation("archive_card"),
            Consequence::SetFlag("has_archive_card", true),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("landing.leave-card"),
        text: "Leave the card in the doorframe and move.",
        target: Transition::Node(StoryNodeId("chapter.stairwell")),
        condition: None,
        effects: &[Consequence::Remember("left_archive_card")],
        intentionally_converges: true,
    },
];

const STAIRWELL_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("stairwell.help-vale"),
        text: "Help Mrs. Vale gather the dropped groceries.",
        target: Transition::Node(StoryNodeId("chapter.street")),
        condition: None,
        effects: &[
            Consequence::AddObservation("helped_vale"),
            Consequence::AdjustRelationship(VALE, 2),
            Consequence::QueueDelayed {
                id: "vale-vouches",
                after_node: StoryNodeId("chapter.diner"),
                effect: DelayedEffect::SetFlag("vale_vouched", true),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("stairwell.take-stairs"),
        text: "Take the stairs before the building changes its mind.",
        target: Transition::Node(StoryNodeId("chapter.street")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(VALE, -1),
            Consequence::Remember("left_vale"),
            Consequence::Shift(Tendency::Agency, 1),
        ],
        intentionally_converges: true,
    },
];

const STREET_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("street.follow-pager"),
        text: "Follow the pager tone toward the diner.",
        target: Transition::Node(StoryNodeId("chapter.diner")),
        condition: None,
        effects: &[
            Consequence::AddObservation("pager_tone"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("street.ask-vendor"),
        text: "Ask the flower seller who has been asking for Mara.",
        target: Transition::Node(StoryNodeId("chapter.diner")),
        condition: None,
        effects: &[
            Consequence::AddObservation("street_rumor"),
            Consequence::Shift(Tendency::Attachment, 1),
        ],
        intentionally_converges: true,
    },
];

const DINER_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("diner.tell-riley"),
        text: "Tell Riley what the room showed you.",
        target: Transition::Node(StoryNodeId("chapter.phone")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(RILEY, 2),
            Consequence::Remember("trusted_riley"),
            Consequence::Shift(Tendency::Attachment, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("diner.test-riley"),
        text: "Ask Riley a question only a stranger would answer wrong.",
        target: Transition::Node(StoryNodeId("chapter.phone")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(RILEY, -1),
            Consequence::Remember("tested_riley"),
            Consequence::Shift(Tendency::Agency, 1),
        ],
        intentionally_converges: true,
    },
];

const PHONE_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("phone.record-message"),
        text: "Write down the caller's exact words.",
        target: Transition::Node(StoryNodeId("chapter.repair-shop")),
        condition: None,
        effects: &[
            Consequence::AddObservation("caller_message"),
            Consequence::QueueDelayed {
                id: "lio-hears-recording",
                after_node: StoryNodeId("chapter.transit"),
                effect: DelayedEffect::AdjustRelationship(LIO, 1),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("phone.hang-up"),
        text: "Hang up before the caller can finish.",
        target: Transition::Node(StoryNodeId("chapter.repair-shop")),
        condition: None,
        effects: &[
            Consequence::Remember("hung_up_on_caller"),
            Consequence::Shift(Tendency::Agency, 1),
        ],
        intentionally_converges: true,
    },
];

const REPAIR_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("repair.ask-lio"),
        text: "Ask Lio to tune the broken pager.",
        target: Transition::Node(StoryNodeId("chapter.transit")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(LIO, 1),
            Consequence::AddObservation("pager_frequency"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("repair.borrow-manual"),
        text: "Borrow the service manual and say less.",
        target: Transition::Node(StoryNodeId("chapter.transit")),
        condition: None,
        effects: &[
            Consequence::AddFact("pager_frequency_is_archival"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
];

const TRANSIT_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("transit.wait"),
        text: "Wait beneath the canceled-route board.",
        target: Transition::Node(StoryNodeId("chapter.disturbance")),
        condition: None,
        effects: &[Consequence::Remember("waited_for_route")],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("transit.walk"),
        text: "Start walking before anyone can redirect you.",
        target: Transition::Node(StoryNodeId("chapter.disturbance")),
        condition: None,
        effects: &[Consequence::Remember("walked_for_route")],
        intentionally_converges: true,
    },
];

const ARCHIVE_LOBBY_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("archive.use-card"),
        text: "Present the appointment card, if it still means anything.",
        target: Transition::Node(StoryNodeId("chapter.archive-stacks")),
        condition: None,
        effects: &[Consequence::Remember("asked_for_archive_entry")],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("archive.ask-public"),
        text: "Ask for the public records desk instead.",
        target: Transition::Node(StoryNodeId("chapter.archive-stacks")),
        condition: None,
        effects: &[Consequence::AddObservation("public_index")],
        intentionally_converges: true,
    },
];

const ARCHIVE_STACKS_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("stacks.read-ledger"),
        text: "Read the handwritten revision ledger.",
        target: Transition::Node(StoryNodeId("chapter.revelation")),
        condition: None,
        effects: &[
            Consequence::AddObservation("revision_ledger"),
            Consequence::AddBelief("archive_is_memory"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("stacks.search-terminal"),
        text: "Search the terminal hidden between the shelves.",
        target: Transition::Node(StoryNodeId("chapter.revelation")),
        condition: None,
        effects: &[
            Consequence::AddObservation("archive_terminal"),
            Consequence::AddBelief("archive_is_machine"),
        ],
        intentionally_converges: true,
    },
];

const REVELATION_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("revelation.call-riley"),
        text: "Call Riley before deciding what the evidence means.",
        target: Transition::Node(StoryNodeId("chapter.turning-point")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(RILEY, 1),
            Consequence::Remember("called_riley_after_revelation"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("revelation.carry-alone"),
        text: "Carry the copied address alone for one more night.",
        target: Transition::Node(StoryNodeId("chapter.turning-point")),
        condition: None,
        effects: &[
            Consequence::AdjustRelationship(RILEY, -1),
            Consequence::Remember("carried_revelation_alone"),
        ],
        intentionally_converges: true,
    },
];

const TURNING_POINT_CHOICES: &[Choice] = &[Choice {
    id: ChoiceId("turning-point.keep-address"),
    text: "Keep the sunset address and choose where to begin tomorrow.",
    target: Transition::Ending(TEMPORARY_ENDING),
    condition: None,
    effects: &[Consequence::AddObservation("sunset_address")],
    intentionally_converges: false,
}];

const CHAPTER_TWO_ADDRESS_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.address.call-riley"),
        text: "Call Riley before the address becomes a private instruction.",
        target: Transition::Node(StoryNodeId("chapter-two.contact")),
        condition: None,
        effects: &[
            Consequence::Remember("chapter_two_called_riley"),
            Consequence::AdjustRelationship(RILEY, 1),
            Consequence::QueueDelayed {
                id: "riley-follows-address",
                after_node: StoryNodeId("chapter-two.exterior"),
                effect: DelayedEffect::SetFlag("riley_followed_address", true),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.address.keep-quiet"),
        text: "Keep the address folded until the city can contradict it.",
        target: Transition::Node(StoryNodeId("chapter-two.contact")),
        condition: None,
        effects: &[
            Consequence::Remember("chapter_two_carried_address"),
            Consequence::QueueDelayed {
                id: "lio-sends-frequency",
                after_node: StoryNodeId("chapter-two.records"),
                effect: DelayedEffect::AddObservation("lio_second_channel"),
            },
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_CONTACT_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.contact.wait-riley"),
        text: "Wait where Riley can find you.",
        target: Transition::Node(StoryNodeId("chapter-two.frequency")),
        condition: None,
        effects: &[
            Consequence::Remember("waited_for_riley_at_sunset"),
            Consequence::AdjustRelationship(RILEY, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.contact.follow-pager"),
        text: "Follow the pager's second pulse without waiting.",
        target: Transition::Node(StoryNodeId("chapter-two.frequency")),
        condition: None,
        effects: &[
            Consequence::Remember("left_before_riley"),
            Consequence::Shift(Tendency::Agency, 1),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_FREQUENCY_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.frequency.ask-lio"),
        text: "Ask Lio what 88.3 indexes when it is not a station.",
        target: Transition::Node(StoryNodeId("chapter-two.records")),
        condition: Some(Condition::Observation("pager_frequency")),
        effects: &[
            Consequence::AddObservation("frequency_is_revision_channel"),
            Consequence::AddActorKnowledge("lio_knows_sunset_location"),
            Consequence::QueueDelayed {
                id: "lio-closes-channel",
                after_node: StoryNodeId("chapter-two.consequence"),
                effect: DelayedEffect::SetFlag("lio_closed_channel", true),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.frequency.read-manual"),
        text: "Read the service manual's erased margin note.",
        target: Transition::Node(StoryNodeId("chapter-two.records")),
        condition: Some(Condition::Fact("pager_frequency_is_archival")),
        effects: &[
            Consequence::AddFact("frequency_is_revision_index"),
            Consequence::Remember("read_second_frequency_alone"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.frequency.trace-caller"),
        text: "Use the caller's exact phrasing as a search key.",
        target: Transition::Node(StoryNodeId("chapter-two.records")),
        condition: Some(Condition::Observation("caller_message")),
        effects: &[
            Consequence::AddObservation("caller_named_revision"),
            Consequence::AddActorBelief("riley_believes_caller_is_future_mara"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_RECORDS_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.records.directory"),
        text: "Check the 1993 city directory.",
        target: Transition::Node(StoryNodeId("chapter-two.route")),
        condition: None,
        effects: &[
            Consequence::AddFact("sunset_lot_17_is_unassigned_1993"),
            Consequence::AddObservation("directory_omits_sunset"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.records.permit"),
        text: "Pull the permit ledger instead.",
        target: Transition::Node(StoryNodeId("chapter-two.route")),
        condition: None,
        effects: &[
            Consequence::AddObservation("permit_names_elias"),
            Consequence::AddActorKnowledge("elias_knows_denied_event"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_ROUTE_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.route.wait-service"),
        text: "Wait for the service shuttle marked off the public map.",
        target: Transition::Node(StoryNodeId("chapter-two.exterior")),
        condition: None,
        effects: &[
            Consequence::Remember("waited_for_sunset_service"),
            Consequence::QueueDelayed {
                id: "service-arrival",
                after_node: StoryNodeId("chapter-two.caretaker"),
                effect: DelayedEffect::AddObservation("riley_arrived_before_mara"),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.route.walk-cut"),
        text: "Take the pedestrian cut through the closed yards.",
        target: Transition::Node(StoryNodeId("chapter-two.exterior")),
        condition: None,
        effects: &[
            Consequence::Remember("walked_to_sunset"),
            Consequence::QueueDelayed {
                id: "walking-arrival",
                after_node: StoryNodeId("chapter-two.caretaker"),
                effect: DelayedEffect::AddObservation("mara_arrived_before_riley"),
            },
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_EXTERIOR_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.exterior.ask-caretaker"),
        text: "Ask the man sweeping an unfinished entrance what stood here.",
        target: Transition::Node(StoryNodeId("chapter-two.caretaker")),
        condition: None,
        effects: &[
            Consequence::AddObservation("caretaker_remembers_fire"),
            Consequence::AddActorBelief("elias_believes_building_was_erased"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.exterior.inspect-facade"),
        text: "Trace the sealed facade before speaking to anyone.",
        target: Transition::Node(StoryNodeId("chapter-two.caretaker")),
        condition: None,
        effects: &[
            Consequence::AddObservation("facade_has_future_bolt_holes"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_CARETAKER_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.caretaker.accept-key"),
        text: "Accept Elias's service key and his version of the fire.",
        target: Transition::Node(StoryNodeId("chapter-two.entry")),
        condition: None,
        effects: &[
            Consequence::Remember("accepted_elias_key"),
            Consequence::AdjustRelationship(ELIAS, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.caretaker.refuse-key"),
        text: "Refuse the key; ask Elias to leave the door unopened.",
        target: Transition::Node(StoryNodeId("chapter-two.entry")),
        condition: None,
        effects: &[
            Consequence::Remember("refused_elias_key"),
            Consequence::AddBelief("elias_may_be_protecting_someone"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_ENTRY_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.entry.service-door"),
        text: "Use the physical service door.",
        target: Transition::Node(StoryNodeId("chapter-two.overlay")),
        condition: None,
        effects: &[
            Consequence::AddObservation("entered_through_1993_door"),
            Consequence::Shift(Tendency::Responsibility, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.entry.projector-door"),
        text: "Align the projector's outline with the missing doorway.",
        target: Transition::Node(StoryNodeId("chapter-two.overlay")),
        condition: None,
        effects: &[
            Consequence::AddObservation("entered_through_revision_outline"),
            Consequence::AddBelief("echo_can_describe_access"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_OVERLAY_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.overlay.inspect-physical-door"),
        text: "Inspect the 1993 door that should not be here.",
        target: Transition::Node(StoryNodeId("chapter-two.disagreement")),
        condition: Some(Condition::EchoLayer(EchoLayer::Physical1993)),
        effects: &[
            Consequence::AddObservation("physical_door_is_locked"),
            Consequence::Remember("compared_physical_door"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.overlay.inspect-revision-door"),
        text: "Inspect the 2013 doorway ECHO insists was open.",
        target: Transition::Node(StoryNodeId("chapter-two.disagreement")),
        condition: Some(Condition::EchoLayer(EchoLayer::Revision2013)),
        effects: &[
            Consequence::AddObservation("revision_door_is_open"),
            Consequence::Remember("compared_revision_door"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_DISAGREEMENT_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.disagreement.keep-physical"),
        text: "Keep the physical cabinet as your evidence.",
        target: Transition::Node(StoryNodeId("chapter-two.personal-record")),
        condition: Some(Condition::EchoLayer(EchoLayer::Physical1993)),
        effects: &[
            Consequence::AddBelief("physical_record_has_priority"),
            Consequence::RemoveBelief("revision_record_has_priority"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.disagreement.keep-revision"),
        text: "Keep the revision's missing cabinet in view.",
        target: Transition::Node(StoryNodeId("chapter-two.personal-record")),
        condition: Some(Condition::EchoLayer(EchoLayer::Revision2013)),
        effects: &[
            Consequence::AddBelief("revision_record_has_priority"),
            Consequence::RemoveBelief("physical_record_has_priority"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_PERSONAL_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.personal.open-card"),
        text: "Open the card addressed to Mara in a year she has not lived.",
        target: Transition::Node(StoryNodeId("chapter-two.intervention")),
        condition: None,
        effects: &[
            Consequence::AddObservation("opened_mara_2013_card"),
            Consequence::AdjustRelationship(RILEY, -1),
            Consequence::QueueDelayed {
                id: "riley-copies-card",
                after_node: StoryNodeId("chapter-two.chamber"),
                effect: DelayedEffect::SetFlag("riley_copied_card", true),
            },
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.personal.leave-card"),
        text: "Leave the card sealed and ask Riley what they remember.",
        target: Transition::Node(StoryNodeId("chapter-two.intervention")),
        condition: None,
        effects: &[
            Consequence::Remember("left_mara_2013_card"),
            Consequence::AdjustRelationship(RILEY, 1),
            Consequence::AddActorBelief("riley_believes_card_is_a_test"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_CHAMBER_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.chamber.read-revisions"),
        text: "Read the sequence of revisions around the same decision.",
        target: Transition::Node(StoryNodeId("chapter-two.predicted-choice")),
        condition: None,
        effects: &[
            Consequence::AddFact("echo_records_observed_decisions"),
            Consequence::AddObservation("seven_mara_revisions"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.chamber.seal-port"),
        text: "Seal the output port before the chamber can send another record.",
        target: Transition::Node(StoryNodeId("chapter-two.predicted-choice")),
        condition: None,
        effects: &[
            Consequence::SetFlag("mara_sealed_output_port", true),
            Consequence::AddBelief("echo_can_be_limited"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_PREDICTED_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.predicted.refuse"),
        text: "Refuse to perform the action ECHO has already annotated.",
        target: Transition::Node(StoryNodeId("chapter-two.response")),
        condition: None,
        effects: &[
            Consequence::Remember("refused_predicted_action"),
            Consequence::Shift(Tendency::Agency, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.predicted.preserve"),
        text: "Preserve the predicted action as evidence without taking it.",
        target: Transition::Node(StoryNodeId("chapter-two.response")),
        condition: None,
        effects: &[
            Consequence::AddObservation("preserved_predicted_action"),
            Consequence::Shift(Tendency::Responsibility, 1),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.predicted.reinterpret"),
        text: "Change what the annotation means before acting beside it.",
        target: Transition::Node(StoryNodeId("chapter-two.response")),
        condition: None,
        effects: &[
            Consequence::AddBelief("prediction_requires_interpretation"),
            Consequence::Shift(Tendency::Curiosity, 1),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_RESPONSE_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.response.disconnect"),
        text: "Disconnect the chamber from the pager relay.",
        target: Transition::Node(StoryNodeId("chapter-two.consequence")),
        condition: None,
        effects: &[
            Consequence::SetFlag("relay_disconnected", true),
            Consequence::Remember("disconnected_relay"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.response.send-name"),
        text: "Send only Mara's name through the relay.",
        target: Transition::Node(StoryNodeId("chapter-two.consequence")),
        condition: None,
        effects: &[
            Consequence::SetFlag("name_sent_to_revision", true),
            Consequence::Remember("sent_name_to_revision"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_DISPLACEMENT_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.displacement.take-cartridge"),
        text: "Carry the warm record cartridge out of the lot.",
        target: Transition::Node(StoryNodeId("chapter-two.turning-point")),
        condition: None,
        effects: &[
            Consequence::SetFlag("mara_kept_revision_cartridge", true),
            Consequence::AddObservation("cartridge_has_2013_reply"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.displacement.leave-cartridge"),
        text: "Leave the cartridge in the room that may have made it.",
        target: Transition::Node(StoryNodeId("chapter-two.turning-point")),
        condition: None,
        effects: &[
            Consequence::Remember("left_revision_cartridge"),
            Consequence::AddBelief("echo_can_reply_without_cartridge"),
        ],
        intentionally_converges: true,
    },
];

const CHAPTER_TWO_TURNING_CHOICES: &[Choice] = &[
    Choice {
        id: ChoiceId("c2.turning.keep-channel"),
        text: "Keep the channel open long enough to hear one more reply.",
        target: Transition::Ending(CHAPTER_TWO_ENDING),
        condition: None,
        effects: &[
            Consequence::AddBelief("someone_in_2013_is_listening"),
            Consequence::Remember("kept_2013_channel_open"),
        ],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("c2.turning.close-notebook"),
        text: "Close the notebook before the reply can name its author.",
        target: Transition::Ending(CHAPTER_TWO_ENDING),
        condition: None,
        effects: &[
            Consequence::AddBelief("echo_wants_mara_to_assume_a_reply"),
            Consequence::Remember("closed_2013_notebook"),
        ],
        intentionally_converges: true,
    },
];

const NODES: &[StoryNode] = &[
    StoryNode {
        id: START_NODE,
        scene: SceneId("bedroom"),
        narration: "You wake beneath a ceiling you remember from somewhere else. The room is holding its breath.",
        choices: NO_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.clock"),
        scene: SceneId("bedroom"),
        narration: "The clock reads 03:17. Below it, the date refuses to be a dream: 1993.",
        choices: CLOCK_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.workstation"),
        scene: SceneId("bedroom"),
        narration: "A beige workstation waits at the desk. Its CRT has already written your name, then erased it.",
        choices: WORKSTATION_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.desk"),
        scene: SceneId("bedroom"),
        narration: "The desk is too orderly. A sealed letter carries your handwriting without your certainty.",
        choices: DESK_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.window"),
        scene: SceneId("bedroom"),
        narration: "Rain holds to the window without falling. Across the street, every dark window faces back.",
        choices: WINDOW_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-clock"),
        scene: SceneId("bedroom"),
        narration: "The digits settle. Something in the room has noticed that you noticed.",
        choices: NO_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-workstation"),
        scene: SceneId("bedroom"),
        narration: "The cursor blinks once more. Whether you read it or not, it has read you.",
        choices: NO_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-desk"),
        scene: SceneId("bedroom"),
        narration: "The paper makes no sound. The room continues as though the decision was made years ago.",
        choices: NO_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.signal"),
        scene: SceneId("bedroom"),
        narration: "The CRT brightens without your touch. A voice arrives through the glass: not yours, almost yours.",
        choices: SIGNAL_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.hallway"),
        scene: SceneId("hallway"),
        narration: "Outside, the hallway has your building's proportions but none of its habits. The exit sign hums in a color you do not remember.",
        choices: HALLWAY_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.kitchen"),
        scene: SceneId("kitchen"),
        narration: "The shared kitchen smells of coffee and rain. A newspaper, a photograph, and three strangers' lunches insist on an ordinary morning.",
        choices: KITCHEN_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.landing"),
        scene: SceneId("landing"),
        narration: "At the building landing, your name is penciled beside an archive appointment. The time is 04:00. Someone expected you awake, not early.",
        choices: LANDING_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.stairwell"),
        scene: SceneId("stairwell"),
        narration: "The stairwell is awake with pipes, footsteps, and Mrs. Vale's grocery bag spilling oranges down the steps.",
        choices: STAIRWELL_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.street"),
        scene: SceneId("street"),
        narration: "Rain moves properly outside. A bus driver argues with a dispatcher, a seller covers flowers with plastic, and the city keeps appointments that have nothing to do with you.",
        choices: STREET_CHOICES,
        entry_effects: &[Consequence::AddObservation("street_is_alive")],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.diner"),
        scene: SceneId("diner"),
        narration: "Riley is waiting in a diner booth beneath a broken clock. They look relieved for exactly one second, then wary of what that relief means.",
        choices: DINER_CHOICES,
        entry_effects: &[Consequence::AddObservation("met_riley")],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.phone"),
        scene: SceneId("phone"),
        narration: "The diner phone rings before either of you can leave. The caller says the archive is not a place so much as a decision that learned to keep records.",
        choices: PHONE_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.repair-shop"),
        scene: SceneId("repair-shop"),
        narration: "Lio's repair shop is bright with solder, cassette decks, and borrowed time. A pager on the counter repeats the frequency from the call.",
        choices: REPAIR_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.transit"),
        scene: SceneId("transit"),
        narration: "At the transit stop, people are reading the cancellation board as if it might apologize. The archive line is still listed, but no vehicle is coming.",
        choices: TRANSIT_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.disturbance"),
        scene: SceneId("transit"),
        narration: "Before you choose a route, archivist Patterson closes the public entrance from across town. It is their decision, made for reasons you cannot yet test. The closure sends everyone, including you, through the service lobby.",
        choices: NO_CHOICES,
        entry_effects: &[
            Consequence::SetFlag("patterson_closed_route", true),
            Consequence::AddFact("patterson_acted_independently"),
            Consequence::AddObservation("archive_route_closed"),
        ],
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(StoryNodeId("chapter.archive-lobby"))),
    },
    StoryNode {
        id: StoryNodeId("chapter.archive-lobby"),
        scene: SceneId("archive-lobby"),
        narration: "The service lobby is open only because the public entrance is not. A clerk recognizes neither your face nor your name, yet lets you toward the stacks after checking an old routing memo.",
        choices: ARCHIVE_LOBBY_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.archive-stacks"),
        scene: SceneId("archive-stacks"),
        narration: "The restricted stacks hold paper records beside a terminal too new for 1993. Both are indexed under a project called ECHO, then crossed out by hand.",
        choices: ARCHIVE_STACKS_CHOICES,
        entry_effects: &[Consequence::AddObservation("echo_project")],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.revelation"),
        scene: SceneId("revelation"),
        narration: "The ledger and terminal agree on one point: ECHO is a future decision system. They disagree about whether it remembers people or predicts them. The photo did not prove your return was planned; it was a test print made after you vanished.",
        choices: REVELATION_CHOICES,
        entry_effects: &[
            Consequence::AddFact("echo_is_future_decision_system"),
            Consequence::AddFact("return_was_not_planned"),
            Consequence::RemoveBelief("return_was_planned"),
            Consequence::AddObservation("contradictory_echo_records"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter.turning-point"),
        scene: SceneId("turning-point"),
        narration: "A copied address points to the place where ECHO will be assembled, years from now. Riley can be beside you or waiting for an explanation, but morning is close and the address is real.",
        choices: TURNING_POINT_CHOICES,
        entry_effects: NO_EFFECTS,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.address"),
        scene: SceneId("c2-address"),
        narration: "At dawn, REVISION 7 / SUNSET LOT 17 / 2013 still occupies the card. The 1993 directory has no street by that name, only a blank edge where the river yards begin.",
        choices: CHAPTER_TWO_ADDRESS_CHOICES,
        entry_effects: &[
            Consequence::AddFact("sunset_address_conflicts_with_1993"),
            Consequence::AddObservation("revision_7_marks_2013"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.contact"),
        scene: SceneId("c2-contact"),
        narration: "Riley answers differently depending on what you asked of them before. They say they remember a lot beside the river. Then, quieter: they do not remember telling you that.",
        choices: CHAPTER_TWO_CONTACT_CHOICES,
        entry_effects: &[
            Consequence::AddActorKnowledge("riley_knows_sunset_location"),
            Consequence::AddObservation("riley_memory_disagrees"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.frequency"),
        scene: SceneId("c2-frequency"),
        narration: "The pager answers on 88.3, then beneath it: a second, dry carrier pulse. It behaves less like a broadcast than a filing instruction.",
        choices: CHAPTER_TWO_FREQUENCY_CHOICES,
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.records"),
        scene: SceneId("c2-records"),
        narration: "City records make Sunset Lot 17 worse. One ledger calls it unassigned. Another calls it a demolished archive annex. Neither can explain a 2013 revision stamp.",
        choices: CHAPTER_TWO_RECORDS_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("city_records_conflict"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.route"),
        scene: SceneId("c2-route"),
        narration: "The canceled line still cuts across the river yards. Waiting feels like accepting its old authority; walking means arriving with less time to be warned.",
        choices: CHAPTER_TWO_ROUTE_CHOICES,
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.exterior"),
        scene: SceneId("c2-exterior"),
        narration: "Sunset Lot 17 is an unfinished facade in 1993: gate, poured foundation, no address plate. Yet orange bolt outlines mark the wall where a later door appears to have been removed.",
        choices: CHAPTER_TWO_EXTERIOR_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("sunset_exterior_incomplete"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.caretaker"),
        scene: SceneId("c2-caretaker"),
        narration: "Elias says he cared for this lot before it existed. He remembers a fire in a room the permit says was never built. He is certain someone survived it; he is not certain who.",
        choices: CHAPTER_TWO_CARETAKER_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("elias_remembers_unbuilt_room"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.entry"),
        scene: SceneId("c2-entry"),
        narration: "The service entrance has a lock, a projected outline, and no innocent way through. Elias steps back. Riley, if present, watches what you decide to call access.",
        choices: CHAPTER_TWO_ENTRY_CHOICES,
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.overlay"),
        scene: SceneId("c2-overlay"),
        narration: "The room accepts two descriptions. In 1993, dust holds a locked cabinet beside a sealed door. In Revision 2013, the cabinet is absent and the doorway is waiting open.",
        choices: CHAPTER_TWO_OVERLAY_CHOICES,
        entry_effects: &[
            Consequence::SetFlag("echo_overlay_unlocked", true),
            Consequence::AddFact("echo_can_render_revision_layer"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.disagreement"),
        scene: SceneId("c2-disagreement"),
        narration: "The cabinet's inventory names a person who is not in the room. The revision omits the cabinet but preserves a footprint in its dust. Both records make the other look edited.",
        choices: CHAPTER_TWO_DISAGREEMENT_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("cabinet_and_revision_disagree"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.personal-record"),
        scene: SceneId("c2-personal-record"),
        narration: "A record card bears Mara's name, a date in 2013, and a note in handwriting that could be hers if memory were a bad witness. Opening it would tell you more and let Riley see less of why you did.",
        choices: CHAPTER_TWO_PERSONAL_CHOICES,
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.intervention"),
        scene: SceneId("c2-intervention"),
        narration: "While you compare the records, Riley makes a decision without asking: they remove a carbon sheet from the card stack. Elias closes the outer gate. Neither action waits for your interpretation.",
        choices: &[Choice {
            id: ChoiceId("c2.intervention.follow"),
            text: "Follow the sound of the revision projector deeper inside.",
            target: Transition::Node(StoryNodeId("chapter-two.chamber")),
            condition: None,
            effects: &[
                Consequence::AddObservation("riley_intervened_independently"),
                Consequence::AddActorKnowledge("riley_has_card_copy"),
                Consequence::SetFlag("elias_closed_outer_gate", true),
            ],
            intentionally_converges: false,
        }],
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.chamber"),
        scene: SceneId("c2-chamber"),
        narration: "The revision chamber does not hold one future. It holds revisions made after a person sees a prediction, then records the changed decision as though it had always been there.",
        choices: CHAPTER_TWO_CHAMBER_CHOICES,
        entry_effects: &[
            Consequence::AddFact("echo_stores_revisions_after_observation"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.predicted-choice"),
        scene: SceneId("c2-predicted-choice"),
        narration: "A projector isolates a line with Mara's name: SEND THE NAME. It offers no date, no source, and no proof that the line predicts anything rather than waiting to be obeyed.",
        choices: CHAPTER_TWO_PREDICTED_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("echo_predicted_send_name"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.response"),
        scene: SceneId("c2-response"),
        narration: "The relay emits a reply whether you disconnect it or not: a page timestamped 2013. The fact of the page is outside Mara's control. Its author is not.",
        choices: CHAPTER_TWO_RESPONSE_CHOICES,
        entry_effects: &[
            Consequence::SetFlag("revision_reply_arrived", true),
            Consequence::AddObservation("reply_exists_in_2013"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.consequence"),
        scene: SceneId("c2-consequence"),
        narration: "The chamber dims. Lio's channel goes silent elsewhere, a precaution they took without telling you. Riley's copy of the card is either proof, protection, or a new way to be wrong.",
        choices: &[Choice {
            id: ChoiceId("c2.consequence.leave"),
            text: "Leave before the lot can revise the exit.",
            target: Transition::Node(StoryNodeId("chapter-two.displacement")),
            condition: None,
            effects: &[
                Consequence::AddObservation("lio_acted_offscreen"),
                Consequence::AddActorBelief("lio_believes_channel_harms_mara"),
            ],
            intentionally_converges: false,
        }],
        entry_effects: &[],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.displacement"),
        scene: SceneId("c2-displacement"),
        narration: "Outside, the lot has not moved, but the facade now carries an orange outline of a room behind it. A warm cartridge waits on the threshold in both descriptions.",
        choices: CHAPTER_TWO_DISPLACEMENT_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("facade_shifted_after_reply"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("chapter-two.turning-point"),
        scene: SceneId("c2-turning-point"),
        narration: "The cartridge or the empty threshold holds a 2013 response: I REMEMBER YOU DIFFERENTLY. It could be an answer from a person. It could be ECHO teaching you to imagine one.",
        choices: CHAPTER_TWO_TURNING_CHOICES,
        entry_effects: &[
            Consequence::AddObservation("chapter_two_2013_response"),
        ],
        uncontrolled_event: false,
        automatic_target: None,
    },
];

const BEDROOM_HOTSPOTS: &[Hotspot] = &[
    Hotspot {
        id: HotspotId::Clock,
        label: "CLOCK",
        target: StoryNodeId("bedroom.clock"),
        condition: None,
    },
    Hotspot {
        id: HotspotId::Workstation,
        label: "WORKSTATION",
        target: StoryNodeId("bedroom.workstation"),
        condition: None,
    },
    Hotspot {
        id: HotspotId::Desk,
        label: "DESK",
        target: StoryNodeId("bedroom.desk"),
        condition: None,
    },
    Hotspot {
        id: HotspotId::Window,
        label: "WINDOW",
        target: StoryNodeId("bedroom.window"),
        condition: Some(Condition::Flag("saw_date", true)),
    },
];

const BEDROOM_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("bedroom.wall"),
        kind: SceneObjectKind::Structural,
        label: "Bedroom wall",
        action: None,
    },
    SceneObject {
        id: ObjectId("bedroom.window"),
        kind: SceneObjectKind::Interactive,
        label: "Window",
        action: None,
    },
    SceneObject {
        id: ObjectId("bedroom.clock"),
        kind: SceneObjectKind::Interactive,
        label: "Clock",
        action: None,
    },
    SceneObject {
        id: ObjectId("bedroom.workstation"),
        kind: SceneObjectKind::Interactive,
        label: "Workstation",
        action: None,
    },
    SceneObject {
        id: ObjectId("bedroom.desk"),
        kind: SceneObjectKind::Interactive,
        label: "Desk",
        action: None,
    },
    SceneObject {
        id: ObjectId("bedroom.lamp"),
        kind: SceneObjectKind::Decorative,
        label: "Desk lamp",
        action: None,
    },
];

const HALLWAY_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("hallway.exit-door"),
        kind: SceneObjectKind::Interactive,
        label: "Exit door",
        action: Some(ChoiceId("hallway.inspect-note")),
    },
    SceneObject {
        id: ObjectId("hallway.note"),
        kind: SceneObjectKind::Interactive,
        label: "Wake note",
        action: Some(ChoiceId("hallway.inspect-note")),
    },
    SceneObject {
        id: ObjectId("hallway.wall-light"),
        kind: SceneObjectKind::Decorative,
        label: "Wall light",
        action: None,
    },
    SceneObject {
        id: ObjectId("hallway.fuse-box"),
        kind: SceneObjectKind::Decorative,
        label: "Fuse box",
        action: None,
    },
];

const KITCHEN_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("kitchen.newspaper"),
        kind: SceneObjectKind::Interactive,
        label: "Tuesday paper",
        action: Some(ChoiceId("kitchen.read-newspaper")),
    },
    SceneObject {
        id: ObjectId("kitchen.photograph"),
        kind: SceneObjectKind::Interactive,
        label: "Photograph",
        action: Some(ChoiceId("kitchen.study-photo")),
    },
    SceneObject {
        id: ObjectId("kitchen.counter"),
        kind: SceneObjectKind::Structural,
        label: "Kitchen counter",
        action: None,
    },
    SceneObject {
        id: ObjectId("kitchen.radio"),
        kind: SceneObjectKind::Decorative,
        label: "Radio",
        action: None,
    },
    SceneObject {
        id: ObjectId("kitchen.cup"),
        kind: SceneObjectKind::Decorative,
        label: "Cup",
        action: None,
    },
];

const LANDING_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("landing.archive-card"),
        kind: SceneObjectKind::Interactive,
        label: "Archive card",
        action: Some(ChoiceId("landing.take-card")),
    },
    SceneObject {
        id: ObjectId("landing.elevator"),
        kind: SceneObjectKind::Structural,
        label: "Elevator door",
        action: None,
    },
    SceneObject {
        id: ObjectId("landing.wall-light"),
        kind: SceneObjectKind::Decorative,
        label: "Wall light",
        action: None,
    },
];

const STAIRWELL_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("stairwell.vale"),
        kind: SceneObjectKind::Actor,
        label: "Mrs. Vale",
        action: Some(ChoiceId("stairwell.help-vale")),
    },
    SceneObject {
        id: ObjectId("stairwell.stairs"),
        kind: SceneObjectKind::Interactive,
        label: "Stairs",
        action: Some(ChoiceId("stairwell.take-stairs")),
    },
    SceneObject {
        id: ObjectId("stairwell.rail"),
        kind: SceneObjectKind::Structural,
        label: "Hand rail",
        action: None,
    },
];

const STREET_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("street.pager"),
        kind: SceneObjectKind::Interactive,
        label: "Pager tone",
        action: Some(ChoiceId("street.follow-pager")),
    },
    SceneObject {
        id: ObjectId("street.flower-seller"),
        kind: SceneObjectKind::Actor,
        label: "Flower seller",
        action: Some(ChoiceId("street.ask-vendor")),
    },
    SceneObject {
        id: ObjectId("street.phone-booth"),
        kind: SceneObjectKind::Decorative,
        label: "Phone booth",
        action: None,
    },
    SceneObject {
        id: ObjectId("street.streetlight"),
        kind: SceneObjectKind::Decorative,
        label: "Streetlight",
        action: None,
    },
];

const DINER_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("diner.riley"),
        kind: SceneObjectKind::Actor,
        label: "Riley",
        action: Some(ChoiceId("diner.tell-riley")),
    },
    SceneObject {
        id: ObjectId("diner.clock"),
        kind: SceneObjectKind::Decorative,
        label: "Broken clock",
        action: None,
    },
    SceneObject {
        id: ObjectId("diner.booth"),
        kind: SceneObjectKind::Structural,
        label: "Booth seven",
        action: None,
    },
    SceneObject {
        id: ObjectId("diner.coffee"),
        kind: SceneObjectKind::Decorative,
        label: "Cold coffee",
        action: None,
    },
];

const PHONE_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("phone.receiver"),
        kind: SceneObjectKind::Stateful,
        label: "Receiver",
        action: Some(ChoiceId("phone.record-message")),
    },
    SceneObject {
        id: ObjectId("phone.keypad"),
        kind: SceneObjectKind::Decorative,
        label: "Keypad",
        action: None,
    },
    SceneObject {
        id: ObjectId("phone.cradle"),
        kind: SceneObjectKind::Structural,
        label: "Phone cradle",
        action: None,
    },
];

const REPAIR_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("repair.lio"),
        kind: SceneObjectKind::Actor,
        label: "Lio",
        action: Some(ChoiceId("repair.ask-lio")),
    },
    SceneObject {
        id: ObjectId("repair.manual"),
        kind: SceneObjectKind::Interactive,
        label: "Service manual",
        action: Some(ChoiceId("repair.borrow-manual")),
    },
    SceneObject {
        id: ObjectId("repair.pager"),
        kind: SceneObjectKind::Stateful,
        label: "Pager",
        action: None,
    },
    SceneObject {
        id: ObjectId("repair.shelves"),
        kind: SceneObjectKind::Structural,
        label: "Parts shelves",
        action: None,
    },
];

const TRANSIT_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("transit.board"),
        kind: SceneObjectKind::Stateful,
        label: "Canceled route board",
        action: Some(ChoiceId("transit.wait")),
    },
    SceneObject {
        id: ObjectId("transit.street"),
        kind: SceneObjectKind::Interactive,
        label: "Walk toward the archive",
        action: Some(ChoiceId("transit.walk")),
    },
    SceneObject {
        id: ObjectId("transit.bench"),
        kind: SceneObjectKind::Decorative,
        label: "Transit bench",
        action: None,
    },
];

const ARCHIVE_LOBBY_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("archive-lobby.card-slot"),
        kind: SceneObjectKind::Stateful,
        label: "Archive card slot",
        action: Some(ChoiceId("archive.use-card")),
    },
    SceneObject {
        id: ObjectId("archive-lobby.clerk"),
        kind: SceneObjectKind::Actor,
        label: "Archive clerk",
        action: Some(ChoiceId("archive.ask-public")),
    },
    SceneObject {
        id: ObjectId("archive-lobby.shelves"),
        kind: SceneObjectKind::Structural,
        label: "Archive shelves",
        action: None,
    },
];

const ARCHIVE_STACKS_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("archive-stacks.ledger"),
        kind: SceneObjectKind::Interactive,
        label: "Revision ledger",
        action: Some(ChoiceId("stacks.read-ledger")),
    },
    SceneObject {
        id: ObjectId("archive-stacks.terminal"),
        kind: SceneObjectKind::Stateful,
        label: "Archive terminal",
        action: Some(ChoiceId("stacks.search-terminal")),
    },
    SceneObject {
        id: ObjectId("archive-stacks.media"),
        kind: SceneObjectKind::Decorative,
        label: "Media cases",
        action: None,
    },
    SceneObject {
        id: ObjectId("archive-stacks.shelves"),
        kind: SceneObjectKind::Structural,
        label: "Restricted shelves",
        action: None,
    },
];

const REVELATION_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("revelation.terminal"),
        kind: SceneObjectKind::Stateful,
        label: "ECHO terminal",
        action: Some(ChoiceId("revelation.call-riley")),
    },
    SceneObject {
        id: ObjectId("revelation.receiver"),
        kind: SceneObjectKind::Interactive,
        label: "Call receiver",
        action: Some(ChoiceId("revelation.call-riley")),
    },
    SceneObject {
        id: ObjectId("revelation.files"),
        kind: SceneObjectKind::Decorative,
        label: "Contradictory files",
        action: None,
    },
];

const TURNING_POINT_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("turning-point.address"),
        kind: SceneObjectKind::Stateful,
        label: "Morning address",
        action: Some(ChoiceId("turning-point.keep-address")),
    },
    SceneObject {
        id: ObjectId("turning-point.revision"),
        kind: SceneObjectKind::Decorative,
        label: "Revision artifact",
        action: None,
    },
    SceneObject {
        id: ObjectId("turning-point.dawn"),
        kind: SceneObjectKind::Decorative,
        label: "Dawn window",
        action: None,
    },
];

const C2_ADDRESS_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-address.artifact"),
        kind: SceneObjectKind::Stateful,
        label: "Revision 7 artifact",
        action: Some(ChoiceId("c2.address.call-riley")),
    },
    SceneObject {
        id: ObjectId("c2-address.directory"),
        kind: SceneObjectKind::Interactive,
        label: "1993 city directory",
        action: Some(ChoiceId("c2.address.keep-quiet")),
    },
];

const C2_CONTACT_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-contact.receiver"),
        kind: SceneObjectKind::Interactive,
        label: "Diner receiver",
        action: Some(ChoiceId("c2.contact.wait-riley")),
    },
    SceneObject {
        id: ObjectId("c2-contact.pager"),
        kind: SceneObjectKind::Stateful,
        label: "Second pulse",
        action: Some(ChoiceId("c2.contact.follow-pager")),
    },
];

const C2_FREQUENCY_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-frequency.pager"),
        kind: SceneObjectKind::Stateful,
        label: "88.3 pager",
        action: Some(ChoiceId("c2.frequency.ask-lio")),
    },
    SceneObject {
        id: ObjectId("c2-frequency.manual"),
        kind: SceneObjectKind::Interactive,
        label: "Service manual",
        action: Some(ChoiceId("c2.frequency.read-manual")),
    },
    SceneObject {
        id: ObjectId("c2-frequency.note"),
        kind: SceneObjectKind::Interactive,
        label: "Caller transcript",
        action: Some(ChoiceId("c2.frequency.trace-caller")),
    },
];

const C2_RECORDS_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-records.directory"),
        kind: SceneObjectKind::Interactive,
        label: "City directory",
        action: Some(ChoiceId("c2.records.directory")),
    },
    SceneObject {
        id: ObjectId("c2-records.permit"),
        kind: SceneObjectKind::Interactive,
        label: "Permit ledger",
        action: Some(ChoiceId("c2.records.permit")),
    },
];

const C2_ROUTE_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-route.shuttle"),
        kind: SceneObjectKind::Interactive,
        label: "Service shuttle board",
        action: Some(ChoiceId("c2.route.wait-service")),
    },
    SceneObject {
        id: ObjectId("c2-route.gate"),
        kind: SceneObjectKind::Interactive,
        label: "Yard gate",
        action: Some(ChoiceId("c2.route.walk-cut")),
    },
];

const C2_EXTERIOR_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-exterior.elias"),
        kind: SceneObjectKind::Actor,
        label: "Elias",
        action: Some(ChoiceId("c2.exterior.ask-caretaker")),
    },
    SceneObject {
        id: ObjectId("c2-exterior.facade"),
        kind: SceneObjectKind::Stateful,
        label: "Unfinished facade",
        action: Some(ChoiceId("c2.exterior.inspect-facade")),
    },
];

const C2_CARETAKER_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-caretaker.key"),
        kind: SceneObjectKind::Interactive,
        label: "Service key",
        action: Some(ChoiceId("c2.caretaker.accept-key")),
    },
    SceneObject {
        id: ObjectId("c2-caretaker.elias"),
        kind: SceneObjectKind::Actor,
        label: "Elias",
        action: Some(ChoiceId("c2.caretaker.refuse-key")),
    },
];

const C2_ENTRY_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-entry.service-door"),
        kind: SceneObjectKind::Interactive,
        label: "Service door",
        action: Some(ChoiceId("c2.entry.service-door")),
    },
    SceneObject {
        id: ObjectId("c2-entry.projector"),
        kind: SceneObjectKind::Stateful,
        label: "Revision projector",
        action: Some(ChoiceId("c2.entry.projector-door")),
    },
];

const C2_OVERLAY_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-overlay.door"),
        kind: SceneObjectKind::Stateful,
        label: "Dual-state door",
        action: None,
    },
    SceneObject {
        id: ObjectId("c2-overlay.cabinet"),
        kind: SceneObjectKind::Stateful,
        label: "Archive cabinet",
        action: None,
    },
    SceneObject {
        id: ObjectId("c2-overlay.projector"),
        kind: SceneObjectKind::Interactive,
        label: "Echo layer switch",
        action: None,
    },
];

const C2_DISAGREEMENT_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-disagreement.cabinet"),
        kind: SceneObjectKind::Stateful,
        label: "Contradictory cabinet",
        action: Some(ChoiceId("c2.disagreement.keep-physical")),
    },
    SceneObject {
        id: ObjectId("c2-disagreement.footprint"),
        kind: SceneObjectKind::Stateful,
        label: "Revision footprint",
        action: Some(ChoiceId("c2.disagreement.keep-revision")),
    },
];

const C2_PERSONAL_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-personal.card"),
        kind: SceneObjectKind::Interactive,
        label: "2013 record card",
        action: Some(ChoiceId("c2.personal.open-card")),
    },
    SceneObject {
        id: ObjectId("c2-personal.riley"),
        kind: SceneObjectKind::Actor,
        label: "Riley",
        action: Some(ChoiceId("c2.personal.leave-card")),
    },
];

const C2_INTERVENTION_OBJECTS: &[SceneObject] = &[SceneObject {
    id: ObjectId("c2-intervention.projector"),
    kind: SceneObjectKind::Stateful,
    label: "Revision projector",
    action: Some(ChoiceId("c2.intervention.follow")),
}];

const C2_CHAMBER_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-chamber.revision-markers"),
        kind: SceneObjectKind::Stateful,
        label: "Revision markers",
        action: Some(ChoiceId("c2.chamber.read-revisions")),
    },
    SceneObject {
        id: ObjectId("c2-chamber.output-port"),
        kind: SceneObjectKind::Interactive,
        label: "Output port",
        action: Some(ChoiceId("c2.chamber.seal-port")),
    },
];

const C2_PREDICTED_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-predicted.annotation"),
        kind: SceneObjectKind::Stateful,
        label: "Predicted annotation",
        action: Some(ChoiceId("c2.predicted.refuse")),
    },
    SceneObject {
        id: ObjectId("c2-predicted.cartridge"),
        kind: SceneObjectKind::Interactive,
        label: "Record cartridge",
        action: Some(ChoiceId("c2.predicted.preserve")),
    },
    SceneObject {
        id: ObjectId("c2-predicted.riley"),
        kind: SceneObjectKind::Actor,
        label: "Riley",
        action: Some(ChoiceId("c2.predicted.reinterpret")),
    },
];

const C2_RESPONSE_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-response.relay"),
        kind: SceneObjectKind::Interactive,
        label: "Pager relay",
        action: Some(ChoiceId("c2.response.disconnect")),
    },
    SceneObject {
        id: ObjectId("c2-response.keyboard"),
        kind: SceneObjectKind::Interactive,
        label: "Name field",
        action: Some(ChoiceId("c2.response.send-name")),
    },
];

const C2_CONSEQUENCE_OBJECTS: &[SceneObject] = &[SceneObject {
    id: ObjectId("c2-consequence.exit"),
    kind: SceneObjectKind::Interactive,
    label: "Service exit",
    action: Some(ChoiceId("c2.consequence.leave")),
}];

const C2_DISPLACEMENT_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-displacement.cartridge"),
        kind: SceneObjectKind::Stateful,
        label: "Warm record cartridge",
        action: Some(ChoiceId("c2.displacement.take-cartridge")),
    },
    SceneObject {
        id: ObjectId("c2-displacement.threshold"),
        kind: SceneObjectKind::Interactive,
        label: "Shifted threshold",
        action: Some(ChoiceId("c2.displacement.leave-cartridge")),
    },
];

const C2_TURNING_OBJECTS: &[SceneObject] = &[
    SceneObject {
        id: ObjectId("c2-turning.reply"),
        kind: SceneObjectKind::Stateful,
        label: "2013 reply",
        action: Some(ChoiceId("c2.turning.keep-channel")),
    },
    SceneObject {
        id: ObjectId("c2-turning.notebook"),
        kind: SceneObjectKind::Interactive,
        label: "Mara's notebook",
        action: Some(ChoiceId("c2.turning.close-notebook")),
    },
];

const C2_OVERLAY_ECHO_OBJECTS: &[EchoObject] = &[
    EchoObject {
        id: ObjectId("c2-overlay.door"),
        layer: EchoObjectLayer::Both,
        label: "Dual-state door",
        action: None,
    },
    EchoObject {
        id: ObjectId("c2-overlay.cabinet"),
        layer: EchoObjectLayer::Physical1993,
        label: "Locked archive cabinet",
        action: Some(ChoiceId("c2.overlay.inspect-physical-door")),
    },
    EchoObject {
        id: ObjectId("c2-overlay.cabinet"),
        layer: EchoObjectLayer::Revision2013,
        label: "Missing cabinet footprint",
        action: Some(ChoiceId("c2.overlay.inspect-revision-door")),
    },
    EchoObject {
        id: ObjectId("c2-overlay.projector"),
        layer: EchoObjectLayer::Both,
        label: "Echo layer switch",
        action: None,
    },
];

const C2_DISAGREEMENT_ECHO_OBJECTS: &[EchoObject] = &[
    EchoObject {
        id: ObjectId("c2-disagreement.cabinet"),
        layer: EchoObjectLayer::Physical1993,
        label: "Inventory cabinet",
        action: Some(ChoiceId("c2.disagreement.keep-physical")),
    },
    EchoObject {
        id: ObjectId("c2-disagreement.footprint"),
        layer: EchoObjectLayer::Revision2013,
        label: "Cabinet footprint",
        action: Some(ChoiceId("c2.disagreement.keep-revision")),
    },
];

const SCENES: &[Scene] = &[
    Scene {
        id: SceneId("bedroom"),
        title: "Bedroom / 03:17",
        hotspots: BEDROOM_HOTSPOTS,
        objects: BEDROOM_OBJECTS,
    },
    Scene {
        id: SceneId("hallway"),
        title: "Hallway / Fourth Floor",
        hotspots: &[],
        objects: HALLWAY_OBJECTS,
    },
    Scene {
        id: SceneId("kitchen"),
        title: "Shared Kitchen / 1993",
        hotspots: &[],
        objects: KITCHEN_OBJECTS,
    },
    Scene {
        id: SceneId("landing"),
        title: "Building Landing / 03:31",
        hotspots: &[],
        objects: LANDING_OBJECTS,
    },
    Scene {
        id: SceneId("stairwell"),
        title: "Stairwell / Downward",
        hotspots: &[],
        objects: STAIRWELL_OBJECTS,
    },
    Scene {
        id: SceneId("street"),
        title: "Rain Street / Before Dawn",
        hotspots: &[],
        objects: STREET_OBJECTS,
    },
    Scene {
        id: SceneId("diner"),
        title: "Cedar Diner / Booth Seven",
        hotspots: &[],
        objects: DINER_OBJECTS,
    },
    Scene {
        id: SceneId("phone"),
        title: "Diner Phone / Incoming",
        hotspots: &[],
        objects: PHONE_OBJECTS,
    },
    Scene {
        id: SceneId("repair-shop"),
        title: "Lio's Repair / Open Late",
        hotspots: &[],
        objects: REPAIR_OBJECTS,
    },
    Scene {
        id: SceneId("transit"),
        title: "Transit Stop / Canceled",
        hotspots: &[],
        objects: TRANSIT_OBJECTS,
    },
    Scene {
        id: SceneId("archive-lobby"),
        title: "City Archive / Service Lobby",
        hotspots: &[],
        objects: ARCHIVE_LOBBY_OBJECTS,
    },
    Scene {
        id: SceneId("archive-stacks"),
        title: "City Archive / Restricted Stacks",
        hotspots: &[],
        objects: ARCHIVE_STACKS_OBJECTS,
    },
    Scene {
        id: SceneId("revelation"),
        title: "ECHO Records / Contradiction",
        hotspots: &[],
        objects: REVELATION_OBJECTS,
    },
    Scene {
        id: SceneId("turning-point"),
        title: "Chapter One / Morning Address",
        hotspots: &[],
        objects: TURNING_POINT_OBJECTS,
    },
    Scene {
        id: SceneId("c2-address"),
        title: "Chapter Two / The Address",
        hotspots: &[],
        objects: C2_ADDRESS_OBJECTS,
    },
    Scene {
        id: SceneId("c2-contact"),
        title: "Cedar Diner / Contact or Silence",
        hotspots: &[],
        objects: C2_CONTACT_OBJECTS,
    },
    Scene {
        id: SceneId("c2-frequency"),
        title: "Lio's Repair / Second Frequency",
        hotspots: &[],
        objects: C2_FREQUENCY_OBJECTS,
    },
    Scene {
        id: SceneId("c2-records"),
        title: "City Records / Missing Address",
        hotspots: &[],
        objects: C2_RECORDS_OBJECTS,
    },
    Scene {
        id: SceneId("c2-route"),
        title: "River Transit / Unlisted Service",
        hotspots: &[],
        objects: C2_ROUTE_OBJECTS,
    },
    Scene {
        id: SceneId("c2-exterior"),
        title: "Sunset Lot 17 / Exterior",
        hotspots: &[],
        objects: C2_EXTERIOR_OBJECTS,
    },
    Scene {
        id: SceneId("c2-caretaker"),
        title: "Sunset Lot 17 / Elias",
        hotspots: &[],
        objects: C2_CARETAKER_OBJECTS,
    },
    Scene {
        id: SceneId("c2-entry"),
        title: "Sunset Lot 17 / Service Entrance",
        hotspots: &[],
        objects: C2_ENTRY_OBJECTS,
    },
    Scene {
        id: SceneId("c2-overlay"),
        title: "Archive Annex / Echo Overlay",
        hotspots: &[],
        objects: C2_OVERLAY_OBJECTS,
    },
    Scene {
        id: SceneId("c2-disagreement"),
        title: "Archive Annex / Contradictory Room",
        hotspots: &[],
        objects: C2_DISAGREEMENT_OBJECTS,
    },
    Scene {
        id: SceneId("c2-personal-record"),
        title: "Archive Annex / Personal Record",
        hotspots: &[],
        objects: C2_PERSONAL_OBJECTS,
    },
    Scene {
        id: SceneId("c2-intervention"),
        title: "Archive Annex / Intervention",
        hotspots: &[],
        objects: C2_INTERVENTION_OBJECTS,
    },
    Scene {
        id: SceneId("c2-chamber"),
        title: "Revision Chamber / Archive Core",
        hotspots: &[],
        objects: C2_CHAMBER_OBJECTS,
    },
    Scene {
        id: SceneId("c2-predicted-choice"),
        title: "Revision Chamber / Annotation",
        hotspots: &[],
        objects: C2_PREDICTED_OBJECTS,
    },
    Scene {
        id: SceneId("c2-response"),
        title: "Revision Chamber / Reply",
        hotspots: &[],
        objects: C2_RESPONSE_OBJECTS,
    },
    Scene {
        id: SceneId("c2-consequence"),
        title: "Archive Annex / Afterimage",
        hotspots: &[],
        objects: C2_CONSEQUENCE_OBJECTS,
    },
    Scene {
        id: SceneId("c2-displacement"),
        title: "Sunset Lot 17 / Shifted Exterior",
        hotspots: &[],
        objects: C2_DISPLACEMENT_OBJECTS,
    },
    Scene {
        id: SceneId("c2-turning-point"),
        title: "Chapter Two / Different Memory",
        hotspots: &[],
        objects: C2_TURNING_OBJECTS,
    },
];

pub fn scenes() -> &'static [Scene] {
    SCENES
}

pub fn nodes() -> &'static [StoryNode] {
    NODES
}

pub fn node(id: StoryNodeId) -> Option<&'static StoryNode> {
    NODES.iter().find(|item| item.id == id)
}

pub fn scene(id: SceneId) -> Option<&'static Scene> {
    SCENES.iter().find(|item| item.id == id)
}

pub fn scene_object(scene_id: SceneId, object_id: ObjectId) -> Option<&'static SceneObject> {
    scene(scene_id)?
        .objects
        .iter()
        .find(|item| item.id == object_id)
}

pub fn scene_supports_echo_overlay(scene_id: SceneId) -> bool {
    matches!(scene_id.0, "c2-overlay" | "c2-disagreement")
}

pub fn echo_objects(scene_id: SceneId) -> &'static [EchoObject] {
    match scene_id.0 {
        "c2-overlay" => C2_OVERLAY_ECHO_OBJECTS,
        "c2-disagreement" => C2_DISAGREEMENT_ECHO_OBJECTS,
        _ => &[],
    }
}

pub fn echo_object_is_active(object: &EchoObject, layer: EchoLayer) -> bool {
    matches!(object.layer, EchoObjectLayer::Both)
        || matches!(
            (object.layer, layer),
            (EchoObjectLayer::Physical1993, EchoLayer::Physical1993)
                | (EchoObjectLayer::Revision2013, EchoLayer::Revision2013)
        )
}

pub fn hotspot(id: HotspotId) -> &'static Hotspot {
    BEDROOM_HOTSPOTS
        .iter()
        .find(|item| item.id == id)
        .unwrap_or(&BEDROOM_HOTSPOTS[0])
}

pub fn actors() -> &'static [ActorId] {
    &[RILEY, VALE, LIO, ELIAS]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldState {
    pub current_node: StoryNodeId,
    pub chapter: u8,
    pub chapter_complete: bool,
    pub echo_layer: EchoLayer,
    pub visited_nodes: BTreeSet<String>,
    pub visit_counts: BTreeMap<String, u16>,
    pub selected_choices: Vec<String>,
    pub flags: StoryFlags,
    pub facts: BTreeSet<String>,
    pub observations: BTreeSet<String>,
    pub beliefs: BTreeSet<String>,
    pub actor_knowledge: BTreeSet<String>,
    pub actor_beliefs: BTreeSet<String>,
    pub memories: BTreeSet<String>,
    pub relationships: BTreeMap<String, i16>,
    pub delayed: Vec<DelayedConsequence>,
    pub tendencies: BTreeMap<String, i16>,
    pub seed: u32,
    pub play_time_ms: u64,
    pub save_generation: u64,
}

pub type GameState = WorldState;

impl WorldState {
    pub fn new() -> Self {
        Self::new_seeded(0x1993_0317)
    }

    pub fn new_seeded(seed: u32) -> Self {
        let mut state = Self {
            current_node: START_NODE,
            chapter: 1,
            chapter_complete: false,
            echo_layer: EchoLayer::Physical1993,
            visited_nodes: BTreeSet::new(),
            visit_counts: BTreeMap::new(),
            selected_choices: Vec::new(),
            flags: StoryFlags::default(),
            facts: BTreeSet::new(),
            observations: BTreeSet::new(),
            beliefs: BTreeSet::new(),
            actor_knowledge: BTreeSet::new(),
            actor_beliefs: BTreeSet::new(),
            memories: BTreeSet::new(),
            relationships: BTreeMap::new(),
            delayed: Vec::new(),
            tendencies: BTreeMap::new(),
            seed,
            play_time_ms: 0,
            save_generation: 0,
        };
        state.mark_visited(START_NODE);
        state
    }

    pub fn mark_visited(&mut self, story_node: StoryNodeId) {
        self.visited_nodes.insert(String::from(story_node.0));
        let count = self.visit_counts.get(story_node.0).copied().unwrap_or(0);
        self.visit_counts
            .insert(String::from(story_node.0), count.saturating_add(1));
    }

    pub fn has_visited(&self, story_node: StoryNodeId) -> bool {
        self.visited_nodes.contains(story_node.0)
    }

    pub fn relationship(&self, actor: ActorId) -> i16 {
        self.relationships.get(actor.0).copied().unwrap_or(0)
    }

    pub fn tendency(&self, tendency: Tendency) -> i16 {
        self.tendencies
            .get(tendency_key(tendency))
            .copied()
            .unwrap_or(0)
    }

    pub fn begin_chapter_two(&mut self) -> Result<(), StoryError> {
        if !self.chapter_complete
            || self.chapter != 1
            || self.current_node != StoryNodeId("chapter.turning-point")
        {
            return Err(StoryError::UnavailableChoice);
        }
        self.chapter = 2;
        self.chapter_complete = false;
        self.echo_layer = EchoLayer::Physical1993;
        self.try_apply_transition(Transition::Node(StoryNodeId("chapter-two.address")))?;
        Ok(())
    }

    pub fn supports_echo_overlay(&self) -> bool {
        node(self.current_node)
            .map(|current| scene_supports_echo_overlay(current.scene))
            .unwrap_or(false)
    }

    pub fn toggle_echo_layer(&mut self) -> bool {
        if !self.supports_echo_overlay() {
            return false;
        }
        self.echo_layer = match self.echo_layer {
            EchoLayer::Physical1993 => EchoLayer::Revision2013,
            EchoLayer::Revision2013 => EchoLayer::Physical1993,
        };
        true
    }

    pub fn available_actions(&self) -> Vec<StoryAction> {
        node(self.current_node)
            .map(|active| {
                active
                    .choices
                    .iter()
                    .filter(|choice| {
                        choice
                            .condition
                            .map(|condition| condition_met(condition, self))
                            .unwrap_or(true)
                    })
                    .map(|choice| StoryAction {
                        id: choice.id,
                        target: choice.target,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn select_choice(&mut self, choice_id: ChoiceId) -> Result<Transition, StoryError> {
        let available = self.available_actions();
        let mut director = ScriptedDirector::choose(choice_id);
        let decision = director.next(self, &available);
        self.apply_director_decision(&available, decision)
    }

    pub fn apply_director_decision(
        &mut self,
        available: &[StoryAction],
        decision: DirectorDecision,
    ) -> Result<Transition, StoryError> {
        let choice_id = decision.action.ok_or(StoryError::InvalidDirectorDecision)?;
        let action = available
            .iter()
            .find(|action| action.id == choice_id)
            .ok_or(StoryError::InvalidDirectorDecision)?;
        if decision.target != Some(action.target) {
            return Err(StoryError::InvalidDirectorDecision);
        }
        let active = node(self.current_node).ok_or(StoryError::UnknownNode)?;
        let choice = active
            .choices
            .iter()
            .find(|choice| choice.id == choice_id)
            .ok_or(StoryError::UnknownChoice)?;
        for effect in choice.effects {
            self.apply_effect(effect);
        }
        if self.selected_choices.len() >= MAX_SELECTED_CHOICES {
            self.selected_choices.remove(0);
        }
        self.selected_choices.push(String::from(choice.id.0));
        if self.chapter == 1 {
            self.memories.insert(String::from(choice.id.0));
        }
        match choice.target {
            Transition::Node(_) => self.try_apply_transition(choice.target)?,
            Transition::Ending(_) => self.chapter_complete = true,
        }
        Ok(choice.target)
    }

    pub fn try_enter_hotspot(&mut self, hotspot_id: HotspotId) -> Result<StoryNodeId, StoryError> {
        let current = node(self.current_node).ok_or(StoryError::UnknownNode)?;
        if current.scene != SceneId("bedroom") {
            return Err(StoryError::UnavailableChoice);
        }
        let selected = hotspot(hotspot_id);
        if !selected
            .condition
            .map(|condition| condition_met(condition, self))
            .unwrap_or(true)
        {
            return Err(StoryError::UnavailableChoice);
        }
        self.try_apply_transition(Transition::Node(selected.target))?;
        Ok(selected.target)
    }

    pub fn enter_hotspot(&mut self, hotspot_id: HotspotId) -> StoryNodeId {
        self.try_enter_hotspot(hotspot_id)
            .unwrap_or(self.current_node)
    }

    pub fn try_apply_transition(&mut self, transition: Transition) -> Result<(), StoryError> {
        if let Transition::Node(next) = transition {
            let next_node = node(next).ok_or(StoryError::InvalidTransition)?;
            self.current_node = next;
            if !scene_supports_echo_overlay(next_node.scene) {
                self.echo_layer = EchoLayer::Physical1993;
            }
            self.mark_visited(next);
            for effect in next_node.entry_effects {
                self.apply_effect(effect);
            }
            self.apply_due_delayed();
        }
        Ok(())
    }

    pub fn apply_transition(&mut self, transition: Transition) {
        let _ = self.try_apply_transition(transition);
    }

    pub fn advance_uncontrolled_event(&mut self) -> Result<Transition, StoryError> {
        let current = node(self.current_node).ok_or(StoryError::UnknownNode)?;
        if !current.uncontrolled_event {
            return Err(StoryError::UnavailableChoice);
        }
        let target = current
            .automatic_target
            .ok_or(StoryError::InvalidTransition)?;
        self.try_apply_transition(target)?;
        Ok(target)
    }

    fn apply_effect(&mut self, effect: &Consequence) {
        match effect {
            Consequence::SetFlag(key, value) => self.flags.set(key, *value),
            Consequence::Shift(tendency, amount) => {
                let key = String::from(tendency_key(*tendency));
                let current = self.tendencies.get(&key).copied().unwrap_or(0);
                self.tendencies
                    .insert(key, current.saturating_add(*amount as i16));
            }
            Consequence::AddFact(value) => {
                self.facts.insert(String::from(*value));
            }
            Consequence::AddObservation(value) => {
                self.observations.insert(String::from(*value));
            }
            Consequence::AddBelief(value) => {
                self.beliefs.insert(String::from(*value));
            }
            Consequence::RemoveBelief(value) => {
                self.beliefs.remove(*value);
            }
            Consequence::AddActorKnowledge(value) => {
                self.actor_knowledge.insert(String::from(*value));
            }
            Consequence::AddActorBelief(value) => {
                self.actor_beliefs.insert(String::from(*value));
            }
            Consequence::Remember(value) => {
                self.memories.insert(String::from(*value));
            }
            Consequence::AdjustRelationship(actor, amount) => {
                let key = String::from(actor.0);
                let current = self.relationships.get(&key).copied().unwrap_or(0);
                self.relationships
                    .insert(key, current.saturating_add(*amount as i16));
            }
            Consequence::QueueDelayed {
                id,
                after_node,
                effect,
            } => {
                if self.delayed.len() < 16 && !self.delayed.iter().any(|item| item.id == *id) {
                    self.delayed.push(DelayedConsequence {
                        id,
                        after_node: *after_node,
                        effect: *effect,
                    });
                }
            }
        }
    }

    fn apply_due_delayed(&mut self) {
        let queued = core::mem::take(&mut self.delayed);
        let mut remaining = Vec::new();
        for delayed in queued {
            if delayed.after_node == self.current_node {
                match delayed.effect {
                    DelayedEffect::SetFlag(key, value) => self.flags.set(key, value),
                    DelayedEffect::AddObservation(value) => {
                        self.observations.insert(String::from(value));
                    }
                    DelayedEffect::AdjustRelationship(actor, amount) => {
                        self.apply_effect(&Consequence::AdjustRelationship(actor, amount));
                    }
                }
            } else {
                remaining.push(delayed);
            }
        }
        self.delayed = remaining;
    }
}

impl Default for WorldState {
    fn default() -> Self {
        Self::new()
    }
}

/// Append a prose fragment so sentences never glue as `anything.Riley`.
///
/// Leading/trailing whitespace on the fragment is normalised.  An explicit
/// separator is inserted when the base ends with sentence punctuation (or any
/// non-whitespace) and the fragment begins with a letter or digit.  Machine
/// syntax such as `ECHO / REVISION` should be authored as a single fragment.
pub fn append_prose(base: &mut String, fragment: &str) {
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return;
    }
    if base.is_empty() {
        base.push_str(fragment);
        return;
    }
    let base_trimmed_end = base.trim_end();
    if base_trimmed_end.len() != base.len() {
        base.truncate(base_trimmed_end.len());
    }
    let needs_space = match (base.chars().last(), fragment.chars().next()) {
        (Some(left), Some(right)) => {
            !left.is_whitespace()
                && !matches!(right, ',' | ';' | ':' | '.' | '!' | '?' | ')' | ']' | '}')
                && (right.is_alphanumeric()
                    || left == '.'
                    || left == '!'
                    || left == '?'
                    || left == ','
                    || left == ';'
                    || left == ':'
                    || left == '\u{2026}')
        }
        _ => false,
    };
    if needs_space {
        base.push(' ');
    }
    base.push_str(fragment);
}

/// Join prose fragments with safe separators.  Empty fragments are skipped.
pub fn join_prose(fragments: &[&str]) -> String {
    let mut out = String::new();
    for fragment in fragments {
        append_prose(&mut out, fragment);
    }
    out
}

/// True when a human-facing prose string contains a suspicious join such as
/// `anything.Riley`.  Intentional machine tokens (`ECHO/REVISION`, IDs, paths)
/// should not be passed here, or should use [`is_machine_syntax`].
pub fn has_suspicious_punctuation_letter_join(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '.' | '!' | '?') {
            if let Some(next) = chars.peek().copied() {
                if next.is_ascii_alphabetic() {
                    return true;
                }
            }
        }
    }
    false
}

/// Short machine-facing labels that intentionally pack punctuation.
pub fn is_machine_syntax(text: &str) -> bool {
    if text.len() > 48 {
        return false;
    }
    if text.contains(" / ") || text.contains("<>") {
        return true;
    }
    let has_machine_sep = text.contains('/') || text.contains('_');
    has_machine_sep
        && text.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, ' ' | '/' | '_' | '-' | '.' | ':' | '<' | '>' | ',')
        })
}

/// Chapter Two ending summary, composed so sentence boundaries keep a space.
pub fn chapter_two_consequence_summary() -> String {
    join_prose(&[
        "Mara has not won or lost anything.",
        "Riley has a copy.",
        "Lio has closed a channel.",
        "Elias remembers a room the city denies.",
        "The response may be from 2013, or from ECHO's need to be believed.",
    ])
}

pub fn presentation_narration(world: &WorldState, story_node: &StoryNode) -> String {
    let mut text = String::from(story_node.narration);
    match story_node.id.0 {
        "chapter.diner" if world.flags.get("vale_vouched") => {
            append_prose(
                &mut text,
                "Mrs. Vale has phoned ahead: \"Tell Riley I saw you leave.\"",
            );
        }
        "chapter.diner" if world.observations.contains("riley_waited") => {
            append_prose(
                &mut text,
                "A second coffee has gone cold beside Riley's hand.",
            );
        }
        "chapter.diner" if world.relationship(RILEY) < 0 => {
            append_prose(
                &mut text,
                "Riley keeps one hand on the exit side of the booth.",
            );
        }
        "chapter.repair-shop" if world.relationship(LIO) > 0 => {
            append_prose(
                &mut text,
                "Lio hears the phrasing you wrote down and unlocks the back cabinet without being asked.",
            );
        }
        "chapter.archive-lobby" if world.flags.get("has_archive_card") => {
            append_prose(
                &mut text,
                "The card's ink has bled into an address the clerk refuses to read aloud.",
            );
        }
        "chapter.turning-point" if world.relationship(RILEY) > 0 => {
            append_prose(
                &mut text,
                "Riley says they will meet you at sunrise, not because they understand, but because they chose to stay.",
            );
        }
        "chapter-two.contact" if world.memories.contains("trusted_riley") => {
            append_prose(
                &mut text,
                "Riley comes close enough to share the pager's light.",
            );
        }
        "chapter-two.contact" if world.memories.contains("tested_riley") => {
            append_prose(
                &mut text,
                "Riley leaves the diner first, then waits across the street where they can deny following.",
            );
        }
        "chapter-two.frequency" if world.observations.contains("caller_message") => {
            append_prose(
                &mut text,
                "Your transcript gives the pulse a phrase to answer.",
            );
        }
        "chapter-two.frequency" if world.facts.contains("pager_frequency_is_archival") => {
            append_prose(
                &mut text,
                "The manual's margin treats 88.3 as an index, not a station.",
            );
        }
        "chapter-two.route" if world.memories.contains("waited_for_route") => {
            append_prose(
                &mut text,
                "You recognize the old instruction to wait, and its power over the morning.",
            );
        }
        "chapter-two.route" if world.memories.contains("walked_for_route") => {
            append_prose(
                &mut text,
                "The cut through the yards feels like refusing the route before it can refuse you.",
            );
        }
        "chapter-two.exterior" if world.flags.get("riley_followed_address") => {
            append_prose(
                &mut text,
                "Riley is already at the gate, which is not the same as arriving with you.",
            );
        }
        "chapter-two.caretaker" if world.observations.contains("riley_arrived_before_mara") => {
            append_prose(
                &mut text,
                "Elias has already spoken to Riley; their versions of the fire do not match.",
            );
        }
        "chapter-two.caretaker" if world.observations.contains("mara_arrived_before_riley") => {
            append_prose(&mut text, "Elias studies you before Riley reaches the lot.");
        }
        "chapter-two.chamber" if world.flags.get("riley_copied_card") => {
            append_prose(
                &mut text,
                "The card stack is lighter. Riley has taken one version of the record with them.",
            );
        }
        "chapter-two.consequence" if world.flags.get("lio_closed_channel") => {
            append_prose(
                &mut text,
                "The pager is quiet in the particular way of a line someone else chose to cut.",
            );
        }
        _ => {}
    }
    text
}

/// Geometry constants for dynamic choice rows.  Pure so layout can be tested
/// without a renderer.
pub const CHOICE_TEXT_INSET_LEFT: i32 = 40;
pub const CHOICE_TEXT_INSET_RIGHT: i32 = 10;
pub const CHOICE_PAD_TOP: i32 = 7;
pub const CHOICE_PAD_BOTTOM: i32 = 7;
pub const CHOICE_LINE_HEIGHT: i32 = 17;
pub const CHOICE_GAP_DEFAULT: i32 = 8;
pub const CHOICE_GAP_MIN: i32 = 4;
pub const CHOICE_MIN_HEIGHT: i32 = 34;
pub const CHOICE_BORDER: i32 = 2;

/// Height of one choice row from its wrapped line count.
pub fn choice_row_height(line_count: usize) -> i32 {
    let lines = line_count.max(1) as i32;
    let content = lines * CHOICE_LINE_HEIGHT;
    let height = CHOICE_PAD_TOP + content + CHOICE_PAD_BOTTOM + CHOICE_BORDER;
    height.max(CHOICE_MIN_HEIGHT)
}

/// Stack choice rows inside a panel.  Gap shrinks toward [`CHOICE_GAP_MIN`]
/// (and to zero if still required) when the rows would otherwise overflow.
pub fn layout_choice_rows(
    line_counts: &[usize],
    panel_x: i32,
    panel_y: i32,
    panel_w: u32,
    panel_h: i32,
) -> alloc::vec::Vec<(i32, i32, u32, u32)> {
    let mut gap = CHOICE_GAP_DEFAULT;
    let heights: alloc::vec::Vec<i32> = line_counts
        .iter()
        .map(|count| choice_row_height(*count))
        .collect();
    let total_content: i32 = heights.iter().sum();
    if line_counts.len() > 1 {
        let gaps = (line_counts.len() - 1) as i32;
        let needed = total_content + gaps * gap;
        if needed > panel_h {
            let spare = panel_h - total_content;
            if spare <= 0 {
                gap = 0;
            } else {
                gap = (spare / gaps).clamp(0, CHOICE_GAP_DEFAULT);
            }
        }
    }
    let mut rows = alloc::vec::Vec::with_capacity(line_counts.len());
    let mut y = panel_y;
    for height in heights {
        rows.push((panel_x, y, panel_w, height as u32));
        y += height + gap;
    }
    rows
}

/// Axis-aligned rectangles used by the Chapter Two turning-point ending at a
/// given window size.  Pure geometry for overlap audits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurningPointLayout {
    pub artifact: (i32, i32, i32, i32),
    pub chapter_title: (i32, i32, i32, i32),
    pub theme_line: (i32, i32, i32, i32),
    pub return_button: (i32, i32, i32, i32),
    pub summary: (i32, i32, i32, i32),
}

fn rect_intersects(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

/// Compute the contemplative ending hierarchy for Chapter Two.
pub fn chapter_two_turning_point_layout(width: u32, height: u32) -> TurningPointLayout {
    let frame_x = 22i32;
    let frame_y = 20i32;
    let frame_w = width.saturating_sub(44) as i32;
    let frame_h = height.saturating_sub(40) as i32;
    let image_x = frame_x + 18;
    let image_y = frame_y + 18;
    let image_w = frame_w - 36;
    let image_h = (frame_h * 57 / 100).max(260);
    let narrative_y = image_y + image_h + 16;
    let narrative_h = (frame_y + frame_h - narrative_y - 18).max(80);
    let artifact_w = 340i32;
    let artifact_h = 52i32;
    let artifact = (
        image_x + image_w / 2 - artifact_w / 2,
        image_y + 48,
        artifact_w,
        artifact_h,
    );
    let chapter_title = (image_x + 40, artifact.1 + artifact.3 + 28, image_w - 80, 28);
    let theme_line = (
        image_x + 48,
        chapter_title.1 + chapter_title.3 + 18,
        image_w - 96,
        22,
    );
    let return_button = (image_x + image_w / 2 - 100, image_y + image_h - 64, 200, 34);
    let summary = (
        image_x + 12,
        narrative_y + 18,
        image_w - 24,
        narrative_h - 28,
    );
    TurningPointLayout {
        artifact,
        chapter_title,
        theme_line,
        return_button,
        summary,
    }
}

/// True when any primary turning-point labels geometrically collide.
pub fn turning_point_layout_has_overlap(layout: &TurningPointLayout) -> bool {
    let rects = [
        layout.artifact,
        layout.chapter_title,
        layout.theme_line,
        layout.return_button,
    ];
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            if rect_intersects(rects[i], rects[j]) {
                return true;
            }
        }
    }
    // Summary lives in the narrative band and must stay below the image chrome.
    rect_intersects(layout.summary, layout.return_button)
        || rect_intersects(layout.summary, layout.theme_line)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryError {
    UnknownNode,
    UnknownChoice,
    UnavailableChoice,
    InvalidTransition,
    InvalidDirectorDecision,
}

fn tendency_key(tendency: Tendency) -> &'static str {
    match tendency {
        Tendency::Agency => "agency",
        Tendency::Responsibility => "responsibility",
        Tendency::Curiosity => "curiosity",
        Tendency::Attachment => "attachment",
    }
}

pub fn condition_met(condition: Condition, state: &WorldState) -> bool {
    match condition {
        Condition::Flag(key, expected) => state.flags.get(key) == expected,
        Condition::Fact(value) => state.facts.contains(value),
        Condition::Observation(value) => state.observations.contains(value),
        Condition::Belief(value) => state.beliefs.contains(value),
        Condition::EchoLayer(layer) => state.echo_layer == layer,
        Condition::Visited(id) => state.has_visited(id),
        Condition::All(items) => items.iter().copied().all(|item| condition_met(item, state)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    DuplicateNode(String),
    DuplicateChoice(String),
    DuplicateObject(String),
    MissingScene(String),
    MissingNode(String),
    MissingActor(String),
    MissingStateKey(String),
    MissingObjectAction(String),
    DecorativeObjectAction(String),
    DeadEnd(String),
    UnmarkedConvergence(String),
    MissingEnding(String),
    UnreachableEnding(String),
    SuspiciousProseJoin(String),
}

pub fn validate_graph() -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let scene_ids: BTreeSet<&str> = SCENES.iter().map(|item| item.id.0).collect();
    let node_ids: BTreeSet<&str> = NODES.iter().map(|item| item.id.0).collect();
    let actor_ids: BTreeSet<&str> = actors().iter().map(|actor| actor.0).collect();
    let mut unique_nodes = BTreeSet::new();
    let mut choice_ids = BTreeSet::new();
    let mut object_ids = BTreeSet::new();
    let mut incoming = BTreeMap::<&str, Vec<&Choice>>::new();
    let mut ending_incoming = BTreeMap::<&str, Vec<&Choice>>::new();

    for scene in SCENES {
        for object in scene.objects {
            if !object_ids.insert(object.id.0) {
                errors.push(ValidationError::DuplicateObject(String::from(object.id.0)));
            }
            if matches!(
                object.kind,
                SceneObjectKind::Structural | SceneObjectKind::Decorative
            ) && object.action.is_some()
            {
                errors.push(ValidationError::DecorativeObjectAction(String::from(
                    object.id.0,
                )));
            }
            if let Some(action) = object.action {
                if !choice_exists(action.0) {
                    errors.push(ValidationError::MissingObjectAction(String::from(
                        object.id.0,
                    )));
                }
            }
        }
        for hotspot in scene.hotspots {
            validate_condition(hotspot.condition, &node_ids, &mut errors);
            if !node_ids.contains(hotspot.target.0) {
                errors.push(ValidationError::MissingNode(String::from(hotspot.target.0)));
            }
        }
    }
    for item in NODES {
        if !unique_nodes.insert(item.id.0) {
            errors.push(ValidationError::DuplicateNode(String::from(item.id.0)));
        }
        if !scene_ids.contains(item.scene.0) {
            errors.push(ValidationError::MissingScene(String::from(item.scene.0)));
        }
        if item.choices.is_empty() && item.automatic_target.is_none() && item.id != START_NODE {
            errors.push(ValidationError::DeadEnd(String::from(item.id.0)));
        }
        if !is_machine_syntax(item.narration)
            && has_suspicious_punctuation_letter_join(item.narration)
        {
            errors.push(ValidationError::SuspiciousProseJoin(String::from(
                item.id.0,
            )));
        }
        validate_effects(item.entry_effects, &node_ids, &actor_ids, &mut errors);
        if let Some(target) = item.automatic_target {
            validate_transition(target, &node_ids, &mut errors);
        }
        for choice in item.choices {
            if !choice_ids.insert(choice.id.0) {
                errors.push(ValidationError::DuplicateChoice(String::from(choice.id.0)));
            }
            if !is_machine_syntax(choice.text)
                && has_suspicious_punctuation_letter_join(choice.text)
            {
                errors.push(ValidationError::SuspiciousProseJoin(String::from(
                    choice.id.0,
                )));
            }
            validate_condition(choice.condition, &node_ids, &mut errors);
            validate_effects(choice.effects, &node_ids, &actor_ids, &mut errors);
            validate_transition(choice.target, &node_ids, &mut errors);
            match choice.target {
                Transition::Node(target) => incoming.entry(target.0).or_default().push(choice),
                Transition::Ending(ending) => {
                    ending_incoming.entry(ending.0).or_default().push(choice)
                }
            }
        }
    }
    let consequence = chapter_two_consequence_summary();
    if has_suspicious_punctuation_letter_join(&consequence) {
        errors.push(ValidationError::SuspiciousProseJoin(String::from(
            "ending.chapter-two.summary",
        )));
    }
    for (target, choices) in incoming {
        if choices.len() > 1 && choices.iter().any(|choice| !choice.intentionally_converges) {
            errors.push(ValidationError::UnmarkedConvergence(String::from(target)));
        }
    }
    for (target, choices) in ending_incoming {
        if choices.len() > 1 && choices.iter().any(|choice| !choice.intentionally_converges) {
            errors.push(ValidationError::UnmarkedConvergence(String::from(target)));
        }
    }
    for ending in [TEMPORARY_ENDING, CHAPTER_TWO_ENDING] {
        if !ending_reachable(ending) {
            errors.push(ValidationError::MissingEnding(String::from(ending.0)));
            errors.push(ValidationError::UnreachableEnding(String::from(ending.0)));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_transition(
    transition: Transition,
    node_ids: &BTreeSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    if let Transition::Node(target) = transition {
        if !node_ids.contains(target.0) {
            errors.push(ValidationError::MissingNode(String::from(target.0)));
        }
    }
}

fn validate_condition(
    condition: Option<Condition>,
    node_ids: &BTreeSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(condition) = condition else { return };
    match condition {
        Condition::Flag(key, _) if !known_flag(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        Condition::Fact(key) if !known_fact(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        Condition::Observation(key) if !known_observation(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        Condition::Belief(key) if !known_belief(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        Condition::EchoLayer(_) => {}
        Condition::Visited(id) if !node_ids.contains(id.0) => {
            errors.push(ValidationError::MissingNode(String::from(id.0)));
        }
        Condition::All(items) => {
            for item in items {
                validate_condition(Some(*item), node_ids, errors);
            }
        }
        _ => {}
    }
}

fn validate_effects(
    effects: &[Consequence],
    node_ids: &BTreeSet<&str>,
    actor_ids: &BTreeSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    for effect in effects {
        match effect {
            Consequence::SetFlag(key, _) if !known_flag(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AddFact(key) if !known_fact(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AddObservation(key) if !known_observation(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AddBelief(key) | Consequence::RemoveBelief(key) if !known_belief(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AddActorKnowledge(key) if !known_actor_knowledge(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AddActorBelief(key) if !known_actor_belief(key) => {
                errors.push(ValidationError::MissingStateKey(String::from(*key)));
            }
            Consequence::AdjustRelationship(actor, _) if !actor_ids.contains(actor.0) => {
                errors.push(ValidationError::MissingActor(String::from(actor.0)));
            }
            Consequence::QueueDelayed {
                id,
                after_node,
                effect,
            } => {
                if delayed_id(id).is_none() || !node_ids.contains(after_node.0) {
                    errors.push(ValidationError::MissingStateKey(String::from(*id)));
                }
                validate_delayed_effect(*effect, actor_ids, errors);
            }
            _ => {}
        }
    }
}

fn validate_delayed_effect(
    effect: DelayedEffect,
    actor_ids: &BTreeSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    match effect {
        DelayedEffect::SetFlag(key, _) if !known_flag(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        DelayedEffect::AddObservation(key) if !known_observation(key) => {
            errors.push(ValidationError::MissingStateKey(String::from(key)));
        }
        DelayedEffect::AdjustRelationship(actor, _) if !actor_ids.contains(actor.0) => {
            errors.push(ValidationError::MissingActor(String::from(actor.0)));
        }
        _ => {}
    }
}

fn known_flag(key: &str) -> bool {
    matches!(
        key,
        "saw_date"
            | "saw_prompt"
            | "opened_letter"
            | "signal_arrived"
            | "has_archive_card"
            | "vale_vouched"
            | "patterson_closed_route"
            | "riley_followed_address"
            | "lio_closed_channel"
            | "echo_overlay_unlocked"
            | "mara_sealed_output_port"
            | "relay_disconnected"
            | "name_sent_to_revision"
            | "riley_copied_card"
            | "elias_closed_outer_gate"
            | "revision_reply_arrived"
            | "mara_kept_revision_cartridge"
    )
}

fn known_fact(key: &str) -> bool {
    matches!(
        key,
        "someone_expected_mara"
            | "year_is_1993"
            | "pager_frequency_is_archival"
            | "patterson_acted_independently"
            | "echo_is_future_decision_system"
            | "return_was_not_planned"
            | "sunset_address_conflicts_with_1993"
            | "frequency_is_revision_index"
            | "sunset_lot_17_is_unassigned_1993"
            | "echo_can_render_revision_layer"
            | "echo_stores_revisions_after_observation"
            | "echo_records_observed_decisions"
    )
}

fn known_observation(key: &str) -> bool {
    matches!(
        key,
        "clock_1993"
            | "waiting_prompt"
            | "letter_in_handwriting"
            | "wake_note"
            | "newspaper_1993"
            | "future_dated_photo"
            | "archive_card"
            | "helped_vale"
            | "street_is_alive"
            | "pager_tone"
            | "street_rumor"
            | "met_riley"
            | "riley_waited"
            | "caller_message"
            | "pager_frequency"
            | "archive_route_closed"
            | "public_index"
            | "echo_project"
            | "revision_ledger"
            | "archive_terminal"
            | "contradictory_echo_records"
            | "sunset_address"
            | "revision_7_marks_2013"
            | "riley_memory_disagrees"
            | "frequency_is_revision_channel"
            | "lio_second_channel"
            | "caller_named_revision"
            | "city_records_conflict"
            | "directory_omits_sunset"
            | "permit_names_elias"
            | "sunset_exterior_incomplete"
            | "caretaker_remembers_fire"
            | "facade_has_future_bolt_holes"
            | "elias_remembers_unbuilt_room"
            | "entered_through_1993_door"
            | "entered_through_revision_outline"
            | "physical_door_is_locked"
            | "revision_door_is_open"
            | "cabinet_and_revision_disagree"
            | "opened_mara_2013_card"
            | "riley_arrived_before_mara"
            | "mara_arrived_before_riley"
            | "riley_intervened_independently"
            | "seven_mara_revisions"
            | "echo_predicted_send_name"
            | "preserved_predicted_action"
            | "reply_exists_in_2013"
            | "lio_acted_offscreen"
            | "facade_shifted_after_reply"
            | "cartridge_has_2013_reply"
            | "chapter_two_2013_response"
    )
}

fn known_belief(key: &str) -> bool {
    matches!(
        key,
        "signal_is_mara"
            | "signal_is_recording"
            | "return_was_planned"
            | "archive_is_memory"
            | "archive_is_machine"
            | "elias_may_be_protecting_someone"
            | "echo_can_describe_access"
            | "physical_record_has_priority"
            | "revision_record_has_priority"
            | "echo_can_be_limited"
            | "prediction_requires_interpretation"
            | "echo_can_reply_without_cartridge"
            | "someone_in_2013_is_listening"
            | "echo_wants_mara_to_assume_a_reply"
    )
}

fn known_actor_knowledge(key: &str) -> bool {
    matches!(
        key,
        "lio_knows_sunset_location"
            | "riley_knows_sunset_location"
            | "elias_knows_denied_event"
            | "riley_has_card_copy"
    )
}

fn known_actor_belief(key: &str) -> bool {
    matches!(
        key,
        "riley_believes_caller_is_future_mara"
            | "elias_believes_building_was_erased"
            | "riley_believes_card_is_a_test"
            | "lio_believes_channel_harms_mara"
    )
}

fn ending_reachable(ending: EndingId) -> bool {
    let mut pending = if ending == CHAPTER_TWO_ENDING {
        Vec::from([StoryNodeId("chapter-two.address")])
    } else {
        Vec::from([START_NODE])
    };
    let mut visited = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current.0) {
            continue;
        }
        let Some(current_node) = node(current) else {
            continue;
        };
        for choice in current_node.choices {
            match choice.target {
                Transition::Ending(found) if found == ending => return true,
                Transition::Node(next) => pending.push(next),
                Transition::Ending(_) => {}
            }
        }
        if let Some(Transition::Node(next)) = current_node.automatic_target {
            pending.push(next);
        }
        if current == START_NODE {
            pending.extend([
                StoryNodeId("bedroom.clock"),
                StoryNodeId("bedroom.workstation"),
                StoryNodeId("bedroom.desk"),
                StoryNodeId("bedroom.window"),
            ]);
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStage {
    Serialize,
    RequestTooLarge,
    TransportSend,
    ServiceDecode,
    ServiceValidate,
    FileOpen,
    FileWrite,
    FileCommit,
    ReplySend,
    ReplyTimeout,
    ReplyMismatch,
    ReplyDecode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    InvalidUtf8,
    UnsupportedVersion,
    InvalidRecord,
    TooLarge,
}

pub fn encode_save(state: &WorldState) -> Vec<u8> {
    let mut out = String::from("SILICON_ECHOES_SAVE\n");
    push_record(&mut out, "version", &format!("{}", SAVE_FORMAT_VERSION));
    push_record(&mut out, "n", state.current_node.0);
    push_record(&mut out, "ch", &format!("{}", state.chapter));
    push_record(
        &mut out,
        "cc",
        if state.chapter_complete { "1" } else { "0" },
    );
    push_record(
        &mut out,
        "el",
        match state.echo_layer {
            EchoLayer::Physical1993 => "1993",
            EchoLayer::Revision2013 => "2013",
        },
    );
    push_record(&mut out, "s", &format!("{}", state.seed));
    push_record(&mut out, "p", &format!("{}", state.play_time_ms));
    push_record(&mut out, "g", &format!("{}", state.save_generation));
    for value in &state.visited_nodes {
        push_record(&mut out, "v", value);
    }
    for (value, count) in &state.visit_counts {
        if *count > 1 {
            push_record(&mut out, "vc", &format!("{}:{}", value, count));
        }
    }
    for value in &state.selected_choices {
        push_record(&mut out, "c", value);
    }
    for (key, value) in state.flags.iter() {
        push_record(
            &mut out,
            "fl",
            &format!("{}:{}", key, if *value { 1 } else { 0 }),
        );
    }
    for value in &state.facts {
        push_record(&mut out, "f", value);
    }
    for value in &state.observations {
        push_record(&mut out, "o", value);
    }
    for value in &state.beliefs {
        push_record(&mut out, "b", value);
    }
    for value in &state.actor_knowledge {
        push_record(&mut out, "ak", value);
    }
    for value in &state.actor_beliefs {
        push_record(&mut out, "ab", value);
    }
    for value in &state.memories {
        push_record(&mut out, "m", value);
    }
    for (actor, trust) in &state.relationships {
        push_record(&mut out, "r", &format!("{}:{}", actor, trust));
    }
    for (key, value) in &state.tendencies {
        push_record(&mut out, "t", &format!("{}:{}", key, value));
    }
    for delayed in &state.delayed {
        push_record(
            &mut out,
            "d",
            &format!(
                "{}:{}:{}",
                delayed.id,
                delayed.after_node.0,
                delayed_code(delayed.effect)
            ),
        );
    }
    out.into_bytes()
}

pub fn decode_save(bytes: &[u8]) -> Result<WorldState, SaveError> {
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(SaveError::TooLarge);
    }
    let text = core::str::from_utf8(bytes).map_err(|_| SaveError::InvalidUtf8)?;
    let mut lines = text.lines();
    if lines.next() != Some("SILICON_ECHOES_SAVE") {
        return Err(SaveError::InvalidRecord);
    }
    let mut records = Vec::new();
    for line in lines {
        if records.len() >= MAX_RECORDS || line.len() > MAX_TEXT_VALUE_BYTES + 24 {
            return Err(SaveError::InvalidRecord);
        }
        let (key, value) = line.split_once('=').ok_or(SaveError::InvalidRecord)?;
        if key.is_empty() || key.len() > 24 || value.len() > MAX_TEXT_VALUE_BYTES {
            return Err(SaveError::InvalidRecord);
        }
        let value = unescape(value)?;
        if value.len() > MAX_TEXT_VALUE_BYTES {
            return Err(SaveError::InvalidRecord);
        }
        records.push((String::from(key), value));
    }
    let version = records
        .iter()
        .find(|(key, _)| key == "version")
        .and_then(|(_, value)| parse_u16(value))
        .ok_or(SaveError::InvalidRecord)?;
    match version {
        1 => decode_v1(&records),
        2 => decode_v2(&records),
        3 => decode_v3(&records),
        SAVE_FORMAT_VERSION => decode_v4(&records),
        _ => Err(SaveError::UnsupportedVersion),
    }
}

fn empty_loaded_state() -> WorldState {
    let mut state = WorldState::new();
    state.visited_nodes.clear();
    state.visit_counts.clear();
    state.selected_choices.clear();
    state.flags = StoryFlags::default();
    state.facts.clear();
    state.observations.clear();
    state.beliefs.clear();
    state.actor_knowledge.clear();
    state.actor_beliefs.clear();
    state.memories.clear();
    state.relationships.clear();
    state.delayed.clear();
    state.tendencies.clear();
    state
}

fn decode_v1(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, false, false, false)?;
    finalize_loaded_state(state)
}

fn decode_v2(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, true, false, false)?;
    finalize_loaded_state(state)
}

fn decode_v3(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, true, true, false)?;
    finalize_loaded_state(state)
}

fn decode_v4(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    let expanded = expand_v4_records(records)?;
    load_common_records(&mut state, &expanded, true, true, true)?;
    finalize_loaded_state(state)
}

fn expand_v4_records(records: &[(String, String)]) -> Result<Vec<(String, String)>, SaveError> {
    let mut expanded = Vec::new();
    for (key, value) in records {
        let full_key = match key.as_str() {
            "version" => "version",
            "n" => "node",
            "ch" => "chapter",
            "cc" => "chapter_complete",
            "el" => "echo_layer",
            "s" => "seed",
            "p" => "play_time_ms",
            "g" => "save_generation",
            "v" => "visited",
            "vc" => "visit_count",
            "c" => "choice",
            "fl" => "flag",
            "f" => "fact",
            "o" => "observation",
            "b" => "belief",
            "ak" => "actor_knowledge",
            "ab" => "actor_belief",
            "m" => "memory",
            "r" => "relationship",
            "t" => "tendency",
            "d" => "delayed",
            _ => return Err(SaveError::InvalidRecord),
        };
        expanded.push((String::from(full_key), value.clone()));
    }
    Ok(expanded)
}

fn load_common_records(
    state: &mut WorldState,
    records: &[(String, String)],
    is_v2: bool,
    has_completion: bool,
    has_chapter_two: bool,
) -> Result<(), SaveError> {
    let mut node_id = None;
    let mut saw_version = false;
    let mut saw_node = false;
    let mut saw_seed = !is_v2;
    let mut saw_play_time = false;
    let mut saw_generation = !has_completion;
    let mut saw_completion = !has_completion;
    let mut saw_chapter = !has_chapter_two;
    let mut saw_echo_layer = !has_chapter_two;
    for (key, value) in records {
        match key.as_str() {
            "version" if !saw_version => saw_version = true,
            "node" if !saw_node => {
                saw_node = true;
                node_id = Some(value.as_str());
            }
            "chapter" if has_chapter_two && !saw_chapter => {
                state.chapter = parse_u16(value).ok_or(SaveError::InvalidRecord)? as u8;
                if !matches!(state.chapter, 1 | 2) {
                    return Err(SaveError::InvalidRecord);
                }
                saw_chapter = true;
            }
            "seed" if is_v2 && !saw_seed => {
                saw_seed = true;
                state.seed = parse_u32(value).ok_or(SaveError::InvalidRecord)?;
            }
            "play_time_ms" => {
                if saw_play_time {
                    return Err(SaveError::InvalidRecord);
                }
                saw_play_time = true;
                state.play_time_ms = parse_u64(value).ok_or(SaveError::InvalidRecord)?
            }
            "save_generation" if has_completion && !saw_generation => {
                saw_generation = true;
                state.save_generation = parse_u64(value).ok_or(SaveError::InvalidRecord)?;
            }
            "chapter_complete" if has_completion && !saw_completion => {
                if !matches!(value.as_str(), "0" | "1") {
                    return Err(SaveError::InvalidRecord);
                }
                saw_completion = true;
                state.chapter_complete = value == "1";
            }
            "echo_layer" if has_chapter_two && !saw_echo_layer => {
                state.echo_layer = match value.as_str() {
                    "1993" => EchoLayer::Physical1993,
                    "2013" => EchoLayer::Revision2013,
                    _ => return Err(SaveError::InvalidRecord),
                };
                saw_echo_layer = true;
            }
            "visited" => {
                if state.visited_nodes.len() >= MAX_VISITED_NODES {
                    return Err(SaveError::InvalidRecord);
                }
                require_node(value)?;
                state.visited_nodes.insert(value.clone());
            }
            "visit_count" if is_v2 => {
                if state.visit_counts.len() >= MAX_VISITED_NODES {
                    return Err(SaveError::InvalidRecord);
                }
                let (node_id, count) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                require_node(node_id)?;
                if state
                    .visit_counts
                    .insert(
                        String::from(node_id),
                        parse_u16(count).ok_or(SaveError::InvalidRecord)?,
                    )
                    .is_some()
                {
                    return Err(SaveError::InvalidRecord);
                }
            }
            "choice" => {
                if !choice_exists(value) {
                    return Err(SaveError::InvalidRecord);
                }
                if state.selected_choices.len() >= MAX_SELECTED_CHOICES {
                    return Err(SaveError::InvalidRecord);
                }
                state.selected_choices.push(value.clone());
            }
            "flag" => {
                if state.flags.values.len() >= MAX_STATE_SET_ITEMS {
                    return Err(SaveError::InvalidRecord);
                }
                let (name, raw) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                if !known_flag(name) || !matches!(raw, "0" | "1") {
                    return Err(SaveError::InvalidRecord);
                }
                state.flags.set(name, raw == "1");
            }
            "fact" if is_v2 => {
                insert_limited_known(&mut state.facts, value, known_fact, MAX_STATE_SET_ITEMS)?
            }
            "observation" if is_v2 => insert_limited_known(
                &mut state.observations,
                value,
                known_observation,
                MAX_STATE_SET_ITEMS,
            )?,
            "belief" if is_v2 => {
                insert_limited_known(&mut state.beliefs, value, known_belief, MAX_STATE_SET_ITEMS)?
            }
            "actor_knowledge" if has_chapter_two => insert_limited_known(
                &mut state.actor_knowledge,
                value,
                known_actor_knowledge,
                MAX_STATE_SET_ITEMS,
            )?,
            "actor_belief" if has_chapter_two => insert_limited_known(
                &mut state.actor_beliefs,
                value,
                known_actor_belief,
                MAX_STATE_SET_ITEMS,
            )?,
            "memory" if is_v2 => {
                if state.memories.len() >= MAX_STATE_SET_ITEMS || value.len() > 96 {
                    return Err(SaveError::InvalidRecord);
                }
                state.memories.insert(value.clone());
            }
            "relationship" if is_v2 => {
                if state.relationships.len() >= MAX_RELATIONSHIPS {
                    return Err(SaveError::InvalidRecord);
                }
                let (actor, amount) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                if !actors().iter().any(|item| item.0 == actor) {
                    return Err(SaveError::InvalidRecord);
                }
                if state
                    .relationships
                    .insert(
                        String::from(actor),
                        parse_i16(amount).ok_or(SaveError::InvalidRecord)?,
                    )
                    .is_some()
                {
                    return Err(SaveError::InvalidRecord);
                }
            }
            "tendency" => {
                if state.tendencies.len() >= MAX_TENDENCIES {
                    return Err(SaveError::InvalidRecord);
                }
                let (name, amount) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                if !matches!(
                    name,
                    "agency" | "responsibility" | "curiosity" | "attachment"
                ) {
                    return Err(SaveError::InvalidRecord);
                }
                if state
                    .tendencies
                    .insert(
                        String::from(name),
                        parse_i16(amount).ok_or(SaveError::InvalidRecord)?,
                    )
                    .is_some()
                {
                    return Err(SaveError::InvalidRecord);
                }
            }
            "delayed" => {
                if state.delayed.len() >= 16 {
                    return Err(SaveError::InvalidRecord);
                }
                let (id, rest) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                let (after_node, code) = rest.split_once(':').ok_or(SaveError::InvalidRecord)?;
                state.delayed.push(DelayedConsequence {
                    id: delayed_id(id).ok_or(SaveError::InvalidRecord)?,
                    after_node: StoryNodeId(require_node(after_node)?),
                    effect: delayed_from_code(code).ok_or(SaveError::InvalidRecord)?,
                });
            }
            _ => return Err(SaveError::InvalidRecord),
        }
    }
    if !saw_version
        || !saw_node
        || !saw_seed
        || !saw_completion
        || !saw_generation
        || !saw_chapter
        || !saw_echo_layer
    {
        return Err(SaveError::InvalidRecord);
    }
    let current = node_id
        .and_then(known_node_id)
        .ok_or(SaveError::InvalidRecord)?;
    state.current_node = StoryNodeId(current);
    Ok(())
}

fn finalize_loaded_state(mut state: WorldState) -> Result<WorldState, SaveError> {
    if state.visited_nodes.is_empty() || !state.visited_nodes.contains(state.current_node.0) {
        return Err(SaveError::InvalidRecord);
    }
    for value in &state.visited_nodes {
        state.visit_counts.entry(value.clone()).or_insert(1);
    }
    if state
        .visit_counts
        .iter()
        .any(|(key, count)| *count == 0 || !state.visited_nodes.contains(key))
    {
        return Err(SaveError::InvalidRecord);
    }
    if state.delayed.iter().enumerate().any(|(index, item)| {
        state.delayed[..index]
            .iter()
            .any(|other| other.id == item.id)
    }) {
        return Err(SaveError::InvalidRecord);
    }
    if state.chapter_complete
        && !matches!(
            state.current_node,
            StoryNodeId("chapter.turning-point") | StoryNodeId("chapter-two.turning-point")
        )
    {
        return Err(SaveError::InvalidRecord);
    }
    if state.chapter == 1 && state.current_node.0.starts_with("chapter-two.") {
        return Err(SaveError::InvalidRecord);
    }
    if state.chapter == 2 && !state.current_node.0.starts_with("chapter-two.") {
        return Err(SaveError::InvalidRecord);
    }
    if state.echo_layer == EchoLayer::Revision2013 && !state.supports_echo_overlay() {
        return Err(SaveError::InvalidRecord);
    }
    Ok(state)
}

fn insert_limited_known(
    values: &mut BTreeSet<String>,
    value: &str,
    known: fn(&str) -> bool,
    limit: usize,
) -> Result<(), SaveError> {
    if !known(value) || values.len() >= limit {
        return Err(SaveError::InvalidRecord);
    }
    values.insert(String::from(value));
    Ok(())
}

fn push_record(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    for byte in value.bytes() {
        match byte {
            b'%' => out.push_str("%25"),
            b'\n' => out.push_str("%0A"),
            b'\r' => out.push_str("%0D"),
            _ => out.push(byte as char),
        }
    }
    out.push('\n');
}

fn unescape(value: &str) -> Result<String, SaveError> {
    let mut out = String::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            out.push(bytes[index] as char);
            index += 1;
            continue;
        }
        let code = bytes
            .get(index + 1..index + 3)
            .ok_or(SaveError::InvalidRecord)?;
        match code {
            b"25" => out.push('%'),
            b"0A" => out.push('\n'),
            b"0D" => out.push('\r'),
            _ => return Err(SaveError::InvalidRecord),
        }
        index += 3;
    }
    Ok(out)
}

fn require_node(id: &str) -> Result<&'static str, SaveError> {
    known_node_id(id).ok_or(SaveError::InvalidRecord)
}

fn known_node_id(id: &str) -> Option<&'static str> {
    NODES
        .iter()
        .find(|item| item.id.0 == id)
        .map(|item| item.id.0)
}

fn choice_exists(id: &str) -> bool {
    NODES
        .iter()
        .flat_map(|item| item.choices)
        .any(|choice| choice.id.0 == id)
}

fn delayed_id(id: &str) -> Option<&'static str> {
    match id {
        "signal-after-window" => Some("signal-after-window"),
        "riley-waited" => Some("riley-waited"),
        "vale-vouches" => Some("vale-vouches"),
        "lio-hears-recording" => Some("lio-hears-recording"),
        "riley-follows-address" => Some("riley-follows-address"),
        "lio-sends-frequency" => Some("lio-sends-frequency"),
        "lio-closes-channel" => Some("lio-closes-channel"),
        "service-arrival" => Some("service-arrival"),
        "walking-arrival" => Some("walking-arrival"),
        "riley-copies-card" => Some("riley-copies-card"),
        _ => None,
    }
}

fn delayed_code(effect: DelayedEffect) -> &'static str {
    match effect {
        DelayedEffect::SetFlag("signal_arrived", true) => "signal-arrived",
        DelayedEffect::SetFlag("vale_vouched", true) => "vale-vouched",
        DelayedEffect::AddObservation("riley_waited") => "riley-waited",
        DelayedEffect::AdjustRelationship(ActorId("lio"), 1) => "lio-recording",
        DelayedEffect::SetFlag("riley_followed_address", true) => "riley-followed",
        DelayedEffect::AddObservation("lio_second_channel") => "lio-second-channel",
        DelayedEffect::SetFlag("lio_closed_channel", true) => "lio-closed-channel",
        DelayedEffect::AddObservation("riley_arrived_before_mara") => "riley-first",
        DelayedEffect::AddObservation("mara_arrived_before_riley") => "mara-first",
        DelayedEffect::SetFlag("riley_copied_card", true) => "riley-copied-card",
        _ => "invalid",
    }
}

fn delayed_from_code(code: &str) -> Option<DelayedEffect> {
    match code {
        "signal-arrived" => Some(DelayedEffect::SetFlag("signal_arrived", true)),
        "vale-vouched" => Some(DelayedEffect::SetFlag("vale_vouched", true)),
        "riley-waited" => Some(DelayedEffect::AddObservation("riley_waited")),
        "lio-recording" => Some(DelayedEffect::AdjustRelationship(LIO, 1)),
        "riley-followed" => Some(DelayedEffect::SetFlag("riley_followed_address", true)),
        "lio-second-channel" => Some(DelayedEffect::AddObservation("lio_second_channel")),
        "lio-closed-channel" => Some(DelayedEffect::SetFlag("lio_closed_channel", true)),
        "riley-first" => Some(DelayedEffect::AddObservation("riley_arrived_before_mara")),
        "mara-first" => Some(DelayedEffect::AddObservation("mara_arrived_before_riley")),
        "riley-copied-card" => Some(DelayedEffect::SetFlag("riley_copied_card", true)),
        _ => None,
    }
}

fn parse_u16(text: &str) -> Option<u16> {
    text.parse().ok()
}
fn parse_u32(text: &str) -> Option<u32> {
    text.parse().ok()
}
fn parse_u64(text: &str) -> Option<u64> {
    text.parse().ok()
}
fn parse_i16(text: &str) -> Option<i16> {
    text.parse().ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StressReport {
    pub completed_runs: u32,
    pub peak_live_states: usize,
}

pub fn run_deterministic_stress(runs: u32) -> Result<StressReport, StoryError> {
    validate_graph().map_err(|_| StoryError::UnknownNode)?;
    let mut peak_live_states = 0;
    for run in 0..runs {
        let mut transient = Vec::new();
        for index in 0..512u32 {
            transient.push(format!(
                "ambient-frame:{}:{}:the-city-keeps-moving",
                run, index
            ));
        }
        peak_live_states = peak_live_states.max(transient.len());
        let mut state = chapter_path(0x1993_0317)?;
        let saved = encode_save(&state);
        state = decode_save(&saved).map_err(|_| StoryError::UnknownNode)?;
        state.select_choice(ChoiceId("turning-point.keep-address"))?;
        drop(transient);
    }
    Ok(StressReport {
        completed_runs: runs,
        peak_live_states,
    })
}

fn chapter_path(seed: u32) -> Result<WorldState, StoryError> {
    let mut state = WorldState::new_seeded(seed);
    state.enter_hotspot(HotspotId::Clock);
    state.select_choice(ChoiceId("clock.accept-date"))?;
    state.advance_uncontrolled_event()?;
    state.enter_hotspot(HotspotId::Workstation);
    state.select_choice(ChoiceId("workstation.read-prompt"))?;
    state.advance_uncontrolled_event()?;
    state.enter_hotspot(HotspotId::Window);
    state.select_choice(ChoiceId("window.answer-signal"))?;
    state.select_choice(ChoiceId("signal-listen"))?;
    state.select_choice(ChoiceId("hallway.inspect-note"))?;
    state.select_choice(ChoiceId("kitchen.study-photo"))?;
    state.select_choice(ChoiceId("landing.take-card"))?;
    state.select_choice(ChoiceId("stairwell.help-vale"))?;
    state.select_choice(ChoiceId("street.follow-pager"))?;
    state.select_choice(ChoiceId("diner.tell-riley"))?;
    state.select_choice(ChoiceId("phone.record-message"))?;
    state.select_choice(ChoiceId("repair.ask-lio"))?;
    state.select_choice(ChoiceId("transit.wait"))?;
    state.advance_uncontrolled_event()?;
    state.select_choice(ChoiceId("archive.use-card"))?;
    state.select_choice(ChoiceId("stacks.read-ledger"))?;
    state.select_choice(ChoiceId("revelation.call-riley"))?;
    Ok(state)
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn presentation_starts_entering_and_hides_choices() {
        let presentation = NarrativePresentation::new(
            String::from("The room waits."),
            100,
            PresentationConfig::default(),
        );
        assert_eq!(presentation.state(), ScenePresentation::Entering);
        assert!(!presentation.choices_visible());
        assert_eq!(presentation.visible_byte_end(), 0);
    }

    #[test]
    fn default_normal_pacing_is_slower_than_legacy() {
        let normal = PresentationProfile::Normal.config();
        assert_eq!(PresentationConfig::default(), normal);
        assert!(normal.grapheme_delay_ms > LEGACY_NORMAL_GRAPHEME_DELAY_MS);
        assert!(normal.sentence_pause_ms > LEGACY_NORMAL_SENTENCE_PAUSE_MS);
        assert!(normal.clause_pause_ms >= 120 && normal.clause_pause_ms <= 180);
        assert!(normal.sentence_pause_ms >= 260 && normal.sentence_pause_ms <= 380);
        assert!(normal.paragraph_pause_ms >= 350 && normal.paragraph_pause_ms <= 500);
        assert!(normal.post_reveal_pause_ms >= 400 && normal.post_reveal_pause_ms <= 650);
        assert!(normal.entrance_pause_ms >= 350 && normal.entrance_pause_ms <= 500);
        assert!(normal.grapheme_delay_ms >= 45 && normal.grapheme_delay_ms <= 55);
        assert!(!normal.instant_text);
        assert!(PresentationProfile::Instant.config().instant_text);
    }

    #[test]
    fn punctuation_pauses_match_profile_delays() {
        let config = PresentationConfig {
            entrance_pause_ms: 0,
            grapheme_delay_ms: 10,
            clause_pause_ms: 40,
            sentence_pause_ms: 80,
            paragraph_pause_ms: 120,
            post_reveal_pause_ms: 50,
            instant_text: false,
        };
        let samples = [
            (",", 40u64),
            (";", 40),
            (":", 40),
            (".", 80),
            ("?", 80),
            ("!", 80),
            ("a\n\n", 120),
            ("word ", 10),
        ];
        for (text, expected) in samples {
            let mut presentation = NarrativePresentation::new(String::from(text), 0, config);
            presentation.tick(0);
            while presentation.revealed_count() < presentation.boundary_count() {
                presentation.tick(u64::MAX / 2);
            }
            assert_eq!(
                presentation.reveal_delay_after_visible(),
                expected,
                "delay mismatch for {text:?}"
            );
        }
    }

    #[test]
    fn presentation_reveals_utf8_at_valid_boundaries_and_then_waits() {
        let config = PresentationConfig {
            entrance_pause_ms: 10,
            grapheme_delay_ms: 1,
            clause_pause_ms: 1,
            sentence_pause_ms: 1,
            paragraph_pause_ms: 1,
            post_reveal_pause_ms: 10,
            instant_text: false,
        };
        let mut presentation = NarrativePresentation::new(String::from("Mara—é."), 0, config);
        assert!(presentation.tick(10));
        assert_eq!(presentation.state(), ScenePresentation::Revealing);
        assert!(presentation.tick(100));
        assert_eq!(presentation.visible_byte_end(), "Mara—é.".len());
        assert_eq!(
            &presentation.text()[..presentation.visible_byte_end()],
            "Mara—é."
        );
        assert_eq!(presentation.state(), ScenePresentation::PostRevealPause);
        assert!(!presentation.choices_visible());
        assert!(presentation.tick(110));
        assert_eq!(presentation.state(), ScenePresentation::AwaitingChoice);
        assert!(presentation.choices_visible());
    }

    #[test]
    fn long_passage_reveals_completely_without_skipping_or_overflow() {
        let mut text = String::new();
        for _ in 0..40 {
            text.push_str("The revision holds a longer memory. ");
        }
        text.push_str("Final.");
        let config = PresentationConfig {
            entrance_pause_ms: 0,
            grapheme_delay_ms: 1,
            clause_pause_ms: 2,
            sentence_pause_ms: 3,
            paragraph_pause_ms: 4,
            post_reveal_pause_ms: 5,
            instant_text: false,
        };
        let mut presentation = NarrativePresentation::new(text.clone(), 0, config);
        presentation.tick(0);
        let mut now = 0u64;
        let mut steps = 0u32;
        while !presentation.choices_visible() && steps < 100_000 {
            now = now.saturating_add(16);
            presentation.tick(now);
            steps += 1;
            let end = presentation.visible_byte_end();
            assert!(text.is_char_boundary(end));
            assert_eq!(&text[..end], &presentation.text()[..end]);
        }
        assert!(presentation.choices_visible());
        assert_eq!(presentation.visible_byte_end(), text.len());
        assert_eq!(presentation.revealed_count(), presentation.boundary_count());
    }

    #[test]
    fn catchup_respects_punctuation_and_caps_work_per_tick() {
        let config = PresentationConfig {
            entrance_pause_ms: 0,
            grapheme_delay_ms: 1,
            clause_pause_ms: 1,
            sentence_pause_ms: 1,
            paragraph_pause_ms: 1,
            post_reveal_pause_ms: 1,
            instant_text: false,
        };
        let long = "a".repeat(MAX_REVEALS_PER_TICK + 40);
        let mut presentation = NarrativePresentation::new(long, 0, config);
        presentation.tick(0);
        presentation.tick(u64::MAX / 4);
        assert!(presentation.revealed_count() <= MAX_REVEALS_PER_TICK);
        assert!(presentation.is_revealing());
        // A second catch-up continues without skipping remaining scalars.
        presentation.tick(u64::MAX / 2);
        assert!(presentation.revealed_count() <= MAX_REVEALS_PER_TICK * 2);
    }

    #[test]
    fn reveal_skip_never_enters_choice_state() {
        let config = PresentationConfig {
            entrance_pause_ms: 0,
            grapheme_delay_ms: 10,
            clause_pause_ms: 10,
            sentence_pause_ms: 10,
            paragraph_pause_ms: 10,
            post_reveal_pause_ms: 20,
            instant_text: false,
        };
        let mut presentation =
            NarrativePresentation::new(String::from("A choice waits."), 0, config);
        presentation.tick(0);
        assert!(presentation.skip_reveal(1));
        assert_eq!(presentation.state(), ScenePresentation::PostRevealPause);
        assert!(!presentation.choices_visible());
        assert_eq!(presentation.visible_byte_end(), presentation.text().len());
    }

    #[test]
    fn space_and_enter_complete_reveal_without_activating_choice() {
        let config = PresentationConfig {
            entrance_pause_ms: 0,
            grapheme_delay_ms: 10,
            clause_pause_ms: 10,
            sentence_pause_ms: 10,
            paragraph_pause_ms: 10,
            post_reveal_pause_ms: 30,
            instant_text: false,
        };
        let mut presentation =
            NarrativePresentation::new(String::from("Wait for the pause."), 0, config);
        presentation.tick(0);
        assert!(presentation.skip_reveal(5));
        assert!(!presentation.choices_visible());
        // The same logical input must not reach AwaitingChoice.
        assert_eq!(presentation.state(), ScenePresentation::PostRevealPause);
        // Post-reveal pause is still active at t=5+29; choices open only after it.
        presentation.tick(5 + 29);
        assert!(!presentation.choices_visible());
        presentation.tick(5 + 30);
        assert!(presentation.choices_visible());
    }

    #[test]
    fn shortcut_letters_remain_ignored_during_reveal() {
        let mut gate = ShortcutGate::default();
        assert_eq!(gate.shortcut_index('a', false, 10), None);
        assert_eq!(gate.shortcut_index('b', false, 20), None);
        assert_eq!(gate.shortcut_index('c', false, 30), None);
        assert_eq!(gate.shortcut_index('d', false, 40), None);
        // Still blocked until quiet release after choices become active.
        assert_eq!(gate.shortcut_index('a', true, 50), None);
        assert_eq!(
            gate.shortcut_index('a', true, 50 + ShortcutGate::QUIET_RELEASE_MS),
            Some(0)
        );
    }

    #[test]
    fn shortcut_gate_blocks_stale_and_repeated_letters() {
        let mut gate = ShortcutGate::default();
        assert_eq!(gate.shortcut_index('a', false, 10), None);
        assert_eq!(gate.shortcut_index('a', true, 20), None);
        assert_eq!(gate.shortcut_index('b', true, 30), Some(1));
        assert_eq!(gate.shortcut_index('b', true, 40), None);
        assert_eq!(gate.shortcut_index('a', true, 300), Some(0));
    }

    #[test]
    fn instant_presentation_is_deterministic_and_noncanonical() {
        let presentation = NarrativePresentation::new(
            String::from("Immediate."),
            77,
            PresentationProfile::Instant.config(),
        );
        assert_eq!(presentation.state(), ScenePresentation::AwaitingChoice);
        assert_eq!(presentation.visible_byte_end(), "Immediate.".len());
        let state = WorldState::new();
        assert_eq!(decode_save(&encode_save(&state)).unwrap(), state);
    }

    #[test]
    fn instant_mode_completes_both_chapters_without_wall_clock_waits() {
        let chapter_one = chapter_path(0x1A).unwrap();
        let mut presentation = NarrativePresentation::new(
            presentation_narration(&chapter_one, node(chapter_one.current_node).unwrap()),
            0,
            PresentationProfile::Instant.config(),
        );
        assert!(presentation.choices_visible());
        assert_eq!(presentation.visible_byte_end(), presentation.text().len());

        let chapter_two = chapter_two_path(0x1A).unwrap();
        presentation = NarrativePresentation::new(
            presentation_narration(&chapter_two, node(chapter_two.current_node).unwrap()),
            0,
            PresentationProfile::Instant.config(),
        );
        assert!(presentation.choices_visible());
        assert!(chapter_two.chapter_complete || chapter_two.current_node.0.contains("turning"));
    }

    #[test]
    fn prose_join_never_glues_period_to_name() {
        let joined = join_prose(&["Mara has not won or lost anything.", "Riley has a copy."]);
        assert_eq!(
            joined,
            "Mara has not won or lost anything. Riley has a copy."
        );
        assert!(!joined.contains("anything.Riley"));
        let mut broken = String::from("anything.");
        append_prose(&mut broken, "Riley");
        assert_eq!(broken, "anything. Riley");
        assert!(!has_suspicious_punctuation_letter_join(&broken));
        assert!(has_suspicious_punctuation_letter_join("anything.Riley"));
        assert!(!has_suspicious_punctuation_letter_join("ECHO / REVISION"));
        assert_eq!(
            chapter_two_consequence_summary(),
            "Mara has not won or lost anything. Riley has a copy. Lio has closed a channel. Elias remembers a room the city denies. The response may be from 2013, or from ECHO's need to be believed."
        );
    }

    #[test]
    fn authored_story_prose_has_no_suspicious_joins() {
        assert_eq!(validate_graph(), Ok(()));
        for item in nodes() {
            assert!(
                !has_suspicious_punctuation_letter_join(item.narration)
                    || is_machine_syntax(item.narration),
                "narration join in {}",
                item.id.0
            );
            for choice in item.choices {
                assert!(
                    !has_suspicious_punctuation_letter_join(choice.text)
                        || is_machine_syntax(choice.text),
                    "choice join in {}",
                    choice.id.0
                );
            }
        }
    }

    #[test]
    fn choice_rows_expand_for_one_two_and_three_lines() {
        assert_eq!(
            choice_row_height(1),
            choice_row_height(1).max(CHOICE_MIN_HEIGHT)
        );
        assert!(choice_row_height(2) > choice_row_height(1));
        assert!(choice_row_height(3) > choice_row_height(2));
        let rows = layout_choice_rows(&[1, 2, 3], 10, 20, 200, 400);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].3, choice_row_height(1) as u32);
        assert_eq!(rows[1].3, choice_row_height(2) as u32);
        assert_eq!(rows[2].3, choice_row_height(3) as u32);
        // Subsequent rows start after the previous measured bottom.
        assert!(rows[1].1 >= rows[0].1 + rows[0].3 as i32);
        assert!(rows[2].1 >= rows[1].1 + rows[1].3 as i32);
        // No overlap.
        assert!(rows[0].1 + rows[0].3 as i32 <= rows[1].1);
        assert!(rows[1].1 + rows[1].3 as i32 <= rows[2].1);
    }

    #[test]
    fn wrapped_choice_hitboxes_match_visual_bounds_and_focus_order() {
        let chamber_second = "Seal the output port before the chamber can send another record.";
        // At typical choice text widths this is two lines; geometry must still
        // use the measured height rather than a fixed one-line row.
        let line_counts = [1usize, 2];
        let rows = layout_choice_rows(&line_counts, 100, 50, 280, 180);
        assert_eq!(rows[0].3, choice_row_height(1) as u32);
        assert_eq!(rows[1].3, choice_row_height(2) as u32);
        assert!(rows[1].3 > rows[0].3);
        assert!(chamber_second.len() > 40);
        // Keyboard order follows visual order: index 0 then 1.
        assert!(rows[0].1 < rows[1].1);
    }

    #[test]
    fn turning_point_layout_elements_do_not_overlap() {
        for (w, h) in [(1080u32, 720u32), (900, 600)] {
            let layout = chapter_two_turning_point_layout(w, h);
            assert!(
                !turning_point_layout_has_overlap(&layout),
                "overlap at {w}x{h}: {layout:?}"
            );
            // Hierarchy: artifact above chapter title above theme above return.
            assert!(layout.artifact.1 < layout.chapter_title.1);
            assert!(layout.chapter_title.1 < layout.theme_line.1);
            assert!(layout.theme_line.1 < layout.return_button.1);
            assert!(layout.summary.1 > layout.return_button.1);
        }
    }

    #[test]
    fn repeated_presentation_rebuild_does_not_accumulate_state() {
        let text = String::from("A short room.");
        for _ in 0..32 {
            let mut presentation =
                NarrativePresentation::new(text.clone(), 0, PresentationProfile::Instant.config());
            assert_eq!(presentation.visible_byte_end(), text.len());
            presentation.begin_transition();
            assert!(!presentation.tick(100));
        }
        let mut gate = ShortcutGate::default();
        for i in 0..16u64 {
            gate.block_until_quiet(i * 10);
            gate.clear();
        }
    }

    fn leave_bedroom(state: &mut WorldState) {
        state.enter_hotspot(HotspotId::Clock);
        state.select_choice(ChoiceId("clock.accept-date")).unwrap();
        state.advance_uncontrolled_event().unwrap();
        state.enter_hotspot(HotspotId::Window);
        state
            .select_choice(ChoiceId("window.answer-signal"))
            .unwrap();
        state.select_choice(ChoiceId("signal-listen")).unwrap();
    }

    #[test]
    fn graph_is_valid_and_all_exposed_actions_are_safe() {
        assert_eq!(validate_graph(), Ok(()));
        for item in nodes() {
            let mut state = WorldState::new();
            state.current_node = item.id;
            for choice in item.choices {
                let available = state.available_actions();
                if available.iter().any(|action| action.id == choice.id) {
                    assert_eq!(state.select_choice(choice.id), Ok(choice.target));
                }
            }
        }
    }

    #[test]
    fn every_reachable_chapter_choice_reaches_implemented_content() {
        let mut completed_paths = 0;
        for first in [ChoiceId("signal.claim-self"), ChoiceId("signal-listen")] {
            explore_chapter_from(signal_ready(first), 0, &mut completed_paths);
        }
        assert!(completed_paths >= 2);
    }

    #[test]
    fn valid_playthrough_has_ten_post_bedroom_scene_transitions() {
        let state = chapter_path(7).unwrap();
        let post_bedroom: BTreeSet<&str> = state
            .visited_nodes
            .iter()
            .filter_map(|id| {
                known_node_id(id)
                    .and_then(|stable| node(StoryNodeId(stable)))
                    .map(|item| item.scene.0)
            })
            .filter(|scene_id| *scene_id != "bedroom")
            .collect();
        assert!(post_bedroom.len() >= 10, "{post_bedroom:?}");
    }

    #[test]
    fn delayed_consequences_schedule_persist_resolve_and_clear() {
        let mut state = WorldState::new();
        leave_bedroom(&mut state);
        state
            .select_choice(ChoiceId("hallway.inspect-note"))
            .unwrap();
        assert_eq!(state.delayed.len(), 1);
        let saved = decode_save(&encode_save(&state)).unwrap();
        assert_eq!(saved.delayed.len(), 1);
        state = saved;
        state
            .select_choice(ChoiceId("kitchen.read-newspaper"))
            .unwrap();
        state.select_choice(ChoiceId("landing.take-card")).unwrap();
        state
            .select_choice(ChoiceId("stairwell.help-vale"))
            .unwrap();
        assert!(!state.flags.get("vale_vouched"));
        assert_eq!(state.delayed.len(), 2);
        state
            .select_choice(ChoiceId("street.follow-pager"))
            .unwrap();
        assert!(state.flags.get("vale_vouched"));
        assert!(state.observations.contains("riley_waited"));
        assert!(state.delayed.is_empty());

        let mut phone_state = WorldState::new();
        leave_bedroom(&mut phone_state);
        phone_state
            .select_choice(ChoiceId("hallway.leave-note"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("kitchen.read-newspaper"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("landing.take-card"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("stairwell.take-stairs"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("street.ask-vendor"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("diner.test-riley"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("phone.record-message"))
            .unwrap();
        assert_eq!(phone_state.delayed.len(), 1);
        phone_state = decode_save(&encode_save(&phone_state)).unwrap();
        phone_state
            .select_choice(ChoiceId("repair.borrow-manual"))
            .unwrap();
        assert_eq!(phone_state.relationship(LIO), 1);
        assert!(phone_state.delayed.is_empty());
        phone_state.select_choice(ChoiceId("transit.walk")).unwrap();
        phone_state.advance_uncontrolled_event().unwrap();
        phone_state
            .select_choice(ChoiceId("archive.ask-public"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("stacks.search-terminal"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("revelation.carry-alone"))
            .unwrap();
        phone_state
            .select_choice(ChoiceId("turning-point.keep-address"))
            .unwrap();
    }

    #[test]
    fn converging_signal_choices_preserve_meaningful_difference() {
        let mut claimed = WorldState::new();
        leave_signal_ready(&mut claimed);
        claimed
            .select_choice(ChoiceId("signal.claim-self"))
            .unwrap();
        let mut listened = WorldState::new();
        leave_signal_ready(&mut listened);
        listened.select_choice(ChoiceId("signal-listen")).unwrap();
        assert_eq!(claimed.current_node, listened.current_node);
        assert_ne!(claimed.beliefs, listened.beliefs);
        assert_ne!(claimed.memories, listened.memories);
    }

    #[test]
    fn independent_closure_changes_later_scene() {
        let state = chapter_path(11).unwrap();
        assert_eq!(state.current_node, StoryNodeId("chapter.turning-point"));
        assert!(state.flags.get("patterson_closed_route"));
        assert!(state.facts.contains("patterson_acted_independently"));
        assert!(state.observations.contains("archive_route_closed"));
    }

    #[test]
    fn save_round_trips_at_required_chapter_points() {
        let mut after_exit = WorldState::new();
        leave_bedroom(&mut after_exit);
        assert_eq!(decode_save(&encode_save(&after_exit)).unwrap(), after_exit);

        let mut pending = after_exit.clone();
        pending
            .select_choice(ChoiceId("hallway.inspect-note"))
            .unwrap();
        assert!(!pending.delayed.is_empty());
        assert_eq!(decode_save(&encode_save(&pending)).unwrap(), pending);

        let near_turning = chapter_path(42).unwrap();
        assert_eq!(
            decode_save(&encode_save(&near_turning)).unwrap(),
            near_turning
        );
    }

    #[test]
    fn save_round_trips_every_reachable_stable_scene_and_branch() {
        let mut completed_paths = 0;
        explore_save_boundaries(WorldState::new(), 0, &mut completed_paths);
        assert!(completed_paths >= 2);
    }

    #[test]
    fn chapter_complete_state_round_trips_and_remains_deterministic() {
        let mut state = chapter_path(0xC0FF_EE).unwrap();
        assert_eq!(
            state.select_choice(ChoiceId("turning-point.keep-address")),
            Ok(Transition::Ending(TEMPORARY_ENDING))
        );
        assert!(state.chapter_complete);
        let loaded = decode_save(&encode_save(&state)).unwrap();
        assert_eq!(loaded, state);
        assert_eq!(loaded.current_node, StoryNodeId("chapter.turning-point"));
    }

    #[test]
    fn converging_state_survives_save_load() {
        let mut claimed = WorldState::new();
        leave_signal_ready(&mut claimed);
        claimed
            .select_choice(ChoiceId("signal.claim-self"))
            .unwrap();
        let claimed = decode_save(&encode_save(&claimed)).unwrap();

        let mut listened = WorldState::new();
        leave_signal_ready(&mut listened);
        listened.select_choice(ChoiceId("signal-listen")).unwrap();
        let listened = decode_save(&encode_save(&listened)).unwrap();

        assert_eq!(claimed.current_node, listened.current_node);
        assert_ne!(claimed.beliefs, listened.beliefs);
        assert_ne!(claimed.memories, listened.memories);
    }

    #[test]
    fn corrupt_truncated_unknown_and_oversized_saves_are_rejected_safely() {
        assert!(matches!(
            decode_save(b"SILICON_ECHOES_SAVE\nversion=3\n"),
            Err(SaveError::InvalidRecord)
        ));
        assert!(matches!(
            decode_save(b"SILICON_ECHOES_SAVE\nversion=99\nnode=bedroom.wake\n"),
            Err(SaveError::UnsupportedVersion)
        ));
        assert!(matches!(
            decode_save(
                b"SILICON_ECHOES_SAVE\nversion=3\nnode=missing\nchapter_complete=0\nseed=1\nplay_time_ms=0\nsave_generation=1\nvisited=bedroom.wake\n"
            ),
            Err(SaveError::InvalidRecord)
        ));
        let oversized = vec![b'x'; MAX_SAVE_BYTES + 1];
        assert_eq!(decode_save(&oversized), Err(SaveError::TooLarge));
    }

    #[test]
    fn scene_objects_are_unique_and_actions_are_valid() {
        assert_eq!(validate_graph(), Ok(()));
        for scene in scenes() {
            let mut ids = BTreeSet::new();
            for object in scene.objects {
                assert!(ids.insert(object.id.0));
                if let Some(action) = object.action {
                    assert!(choice_exists(action.0));
                    assert!(!matches!(
                        object.kind,
                        SceneObjectKind::Structural | SceneObjectKind::Decorative
                    ));
                }
            }
        }
    }

    #[test]
    fn version_one_bedroom_saves_migrate_without_reinterpreting_unknown_data() {
        let v1 = b"SILICON_ECHOES_SAVE\n\
version=1\n\
node=bedroom.window\n\
play_time_ms=42\n\
visited=bedroom.wake\n\
visited=bedroom.window\n\
choice=clock.accept-date\n\
flag=saw_date:1\n\
tendency=responsibility:1\n";
        let loaded = decode_save(v1).unwrap();
        assert_eq!(loaded.current_node, StoryNodeId("bedroom.window"));
        assert_eq!(loaded.play_time_ms, 42);
        assert!(loaded.flags.get("saw_date"));
        assert_eq!(loaded.visit_counts.get("bedroom.window"), Some(&1));
        assert!(decode_save(b"SILICON_ECHOES_SAVE\nversion=1\nnode=missing\n").is_err());
    }

    #[test]
    fn deterministic_replay_is_identical() {
        assert_eq!(
            chapter_path(0xCAFE_BABE).unwrap(),
            chapter_path(0xCAFE_BABE).unwrap()
        );
    }

    #[test]
    fn stress_releases_prior_scene_state() {
        assert_eq!(run_deterministic_stress(64).unwrap().completed_runs, 64);
    }

    #[test]
    fn save_sizes_remain_within_transport_page() {
        let state = chapter_path(0x1993_0317).unwrap();
        let size = encode_save(&state).len();
        std::println!(
            "[SAVE-SIZE] turning-point state: {} bytes (limit 4096)",
            size
        );
        std::println!(
            "[SAVE-SIZE] visited_nodes={} visit_counts={} selected_choices={}",
            state.visited_nodes.len(),
            state.visit_counts.len(),
            state.selected_choices.len()
        );
        std::println!(
            "[SAVE-SIZE] flags={} facts={} observations={} beliefs={} memories={}",
            state.flags.iter().count(),
            state.facts.len(),
            state.observations.len(),
            state.beliefs.len(),
            state.memories.len()
        );
        std::println!(
            "[SAVE-SIZE] relationships={} tendencies={} delayed={}",
            state.relationships.len(),
            state.tendencies.len(),
            state.delayed.len()
        );
        let mut complete = state.clone();
        complete
            .select_choice(ChoiceId("turning-point.keep-address"))
            .unwrap();
        let csize = encode_save(&complete).len();
        std::println!("[SAVE-SIZE] chapter-complete state: {} bytes", csize);
        std::println!(
            "[SAVE-SIZE] complete visited={} choices={} observations={} memories={}",
            complete.visited_nodes.len(),
            complete.selected_choices.len(),
            complete.observations.len(),
            complete.memories.len()
        );
        assert_eq!(
            decode_save(&encode_save(&state)).unwrap(),
            state,
            "chapter-path state round-trips"
        );
        assert!(
            size <= 4096,
            "encoded save at turning-point ({size} bytes) must not exceed the 4096-byte SHM transport page"
        );
    }

    #[test]
    fn save_sizes_at_every_scene() {
        let mut state = WorldState::new_seeded(0x1993_0317);
        let mut first_over = None;
        let mut peak = 0usize;
        let mut size_at = |s: &WorldState, stage: &str| {
            let sz = encode_save(s).len();
            if sz > peak {
                peak = sz;
            }
            if sz > 4096 && first_over.is_none() {
                first_over = Some((String::from(stage), sz));
            }
        };
        size_at(&state, "bedroom.wake");
        state.enter_hotspot(HotspotId::Clock);
        size_at(&state, "bedroom.clock");
        state.select_choice(ChoiceId("clock.accept-date")).unwrap();
        size_at(&state, "clock.accept-date");
        state.advance_uncontrolled_event().unwrap();
        size_at(&state, "back-from-clock");
        state.enter_hotspot(HotspotId::Workstation);
        size_at(&state, "bedroom.workstation");
        state
            .select_choice(ChoiceId("workstation.read-prompt"))
            .unwrap();
        size_at(&state, "read-prompt");
        state.advance_uncontrolled_event().unwrap();
        size_at(&state, "back-from-ws");
        state.enter_hotspot(HotspotId::Window);
        size_at(&state, "bedroom.window");
        state
            .select_choice(ChoiceId("window.answer-signal"))
            .unwrap();
        size_at(&state, "answer-signal");
        state.select_choice(ChoiceId("signal-listen")).unwrap();
        size_at(&state, "signal-listen");
        state
            .select_choice(ChoiceId("hallway.inspect-note"))
            .unwrap();
        size_at(&state, "hallway.inspect-note");
        state
            .select_choice(ChoiceId("kitchen.study-photo"))
            .unwrap();
        size_at(&state, "kitchen.study-photo");
        state.select_choice(ChoiceId("landing.take-card")).unwrap();
        size_at(&state, "landing.take-card");
        state
            .select_choice(ChoiceId("stairwell.help-vale"))
            .unwrap();
        size_at(&state, "stairwell.help-vale");
        state
            .select_choice(ChoiceId("street.follow-pager"))
            .unwrap();
        size_at(&state, "street.follow-pager");
        state.select_choice(ChoiceId("diner.tell-riley")).unwrap();
        size_at(&state, "diner.tell-riley");
        state
            .select_choice(ChoiceId("phone.record-message"))
            .unwrap();
        size_at(&state, "phone.record-message");
        state.select_choice(ChoiceId("repair.ask-lio")).unwrap();
        size_at(&state, "repair.ask-lio");
        state.select_choice(ChoiceId("transit.wait")).unwrap();
        size_at(&state, "transit.wait");
        state.advance_uncontrolled_event().unwrap();
        size_at(&state, "disturbance");
        state.select_choice(ChoiceId("archive.use-card")).unwrap();
        size_at(&state, "archive.use-card");
        state.select_choice(ChoiceId("stacks.read-ledger")).unwrap();
        size_at(&state, "stacks.read-ledger");
        state
            .select_choice(ChoiceId("revelation.call-riley"))
            .unwrap();
        size_at(&state, "revelation.call-riley");
        size_at(&state, "turning-point");
        state
            .select_choice(ChoiceId("turning-point.keep-address"))
            .unwrap();
        size_at(&state, "chapter-complete");
        assert!(
            peak <= 4096,
            "peak save size ({peak} bytes) must stay within SHM transport page (4096). {}",
            first_over.map_or(String::new(), |(stage, sz)| format!(
                "First over at {stage}: {sz} bytes"
            ))
        );
        let final_state = chapter_path(0x1993_0317).unwrap();
        assert_eq!(
            decode_save(&encode_save(&final_state)).unwrap(),
            final_state
        );
    }

    #[test]
    fn every_chapter_one_completion_variant_enters_chapter_two_without_resetting_state() {
        for final_choice in [
            ChoiceId("revelation.call-riley"),
            ChoiceId("revelation.carry-alone"),
        ] {
            let mut state = chapter_path_with_revelation(final_choice).unwrap();
            let chapter_one_memories = state.memories.clone();
            state
                .select_choice(ChoiceId("turning-point.keep-address"))
                .unwrap();
            assert!(state.chapter_complete);
            let boundary = decode_save(&encode_save(&state)).unwrap();
            state = boundary;
            state.begin_chapter_two().unwrap();
            assert_eq!(state.chapter, 2);
            assert_eq!(state.current_node, StoryNodeId("chapter-two.address"));
            assert!(!state.chapter_complete);
            assert!(state.observations.contains("sunset_address"));
            assert_eq!(
                state.memories,
                chapter_one_memories
                    .union(&BTreeSet::from([String::from(
                        "turning-point.keep-address"
                    )]))
                    .cloned()
                    .collect()
            );
        }
    }

    #[test]
    fn chapter_two_normal_path_has_twelve_genuine_transitions_before_turning_point() {
        let state = chapter_two_path(0x1993_0317).unwrap();
        let chapter_two_scenes: BTreeSet<&str> = state
            .visited_nodes
            .iter()
            .filter_map(|id| known_node_id(id))
            .filter_map(|id| node(StoryNodeId(id)))
            .filter(|item| item.id.0.starts_with("chapter-two."))
            .map(|item| item.scene.0)
            .collect();
        assert_eq!(state.current_node, StoryNodeId("chapter-two.turning-point"));
        assert!(
            chapter_two_scenes.len() >= 14,
            "normal Chapter Two path must enter distinct scenes: {chapter_two_scenes:?}"
        );
    }

    #[test]
    fn all_exposed_chapter_two_actions_reach_implemented_content() {
        let mut completed_paths = 0;
        explore_chapter_two_from(chapter_two_entry(0xABCD).unwrap(), 0, &mut completed_paths);
        assert!(completed_paths >= 12);
    }

    #[test]
    fn chapter_one_choices_change_chapter_two_presentation_state() {
        let trusted = chapter_two_entry_with(
            ChoiceId("diner.tell-riley"),
            ChoiceId("phone.record-message"),
            ChoiceId("repair.ask-lio"),
            ChoiceId("transit.wait"),
            ChoiceId("revelation.call-riley"),
        )
        .unwrap();
        let guarded = chapter_two_entry_with(
            ChoiceId("diner.test-riley"),
            ChoiceId("phone.hang-up"),
            ChoiceId("repair.borrow-manual"),
            ChoiceId("transit.walk"),
            ChoiceId("revelation.carry-alone"),
        )
        .unwrap();
        assert_ne!(trusted.relationship(RILEY), guarded.relationship(RILEY));
        assert_ne!(
            trusted.observations.contains("caller_message"),
            guarded.observations.contains("caller_message")
        );
        assert_ne!(
            trusted.observations.contains("pager_frequency"),
            guarded.observations.contains("pager_frequency")
        );
        assert_ne!(
            trusted.memories.contains("waited_for_route"),
            guarded.memories.contains("waited_for_route")
        );
        assert_ne!(
            trusted.memories.contains("called_riley_after_revelation"),
            guarded.memories.contains("called_riley_after_revelation")
        );
    }

    #[test]
    fn chapter_two_delayed_consequences_persist_and_resolve_once() {
        let mut state = chapter_two_entry(0xD1A1).unwrap();
        state
            .select_choice(ChoiceId("c2.address.keep-quiet"))
            .unwrap();
        assert_eq!(state.delayed.len(), 1);
        state = decode_save(&encode_save(&state)).unwrap();
        state
            .select_choice(ChoiceId("c2.contact.follow-pager"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.frequency.ask-lio"))
            .unwrap();
        assert_eq!(state.delayed.len(), 1);
        state
            .select_choice(ChoiceId("c2.records.directory"))
            .unwrap();
        assert!(state.observations.contains("lio_second_channel"));
        state.select_choice(ChoiceId("c2.route.walk-cut")).unwrap();
        assert_eq!(state.delayed.len(), 2);
        state
            .select_choice(ChoiceId("c2.exterior.ask-caretaker"))
            .unwrap();
        assert!(state.observations.contains("mara_arrived_before_riley"));
        state
            .select_choice(ChoiceId("c2.caretaker.accept-key"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.entry.service-door"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.overlay.inspect-physical-door"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.disagreement.keep-physical"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.personal.open-card"))
            .unwrap();
        assert_eq!(state.delayed.len(), 2);
        state
            .select_choice(ChoiceId("c2.intervention.follow"))
            .unwrap();
        assert!(state.flags.get("riley_copied_card"));
        state
            .select_choice(ChoiceId("c2.chamber.read-revisions"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.predicted.preserve"))
            .unwrap();
        state
            .select_choice(ChoiceId("c2.response.disconnect"))
            .unwrap();
        assert!(state.flags.get("lio_closed_channel"));
        assert!(state.delayed.is_empty());
    }

    #[test]
    fn echo_overlay_is_layered_saveable_and_does_not_duplicate_actions() {
        let mut state = chapter_two_path_to_overlay(0xEC40).unwrap();
        assert!(state.supports_echo_overlay());
        assert_eq!(state.echo_layer, EchoLayer::Physical1993);
        assert!(state
            .available_actions()
            .iter()
            .any(|action| action.id == ChoiceId("c2.overlay.inspect-physical-door")));
        assert!(!state
            .available_actions()
            .iter()
            .any(|action| action.id == ChoiceId("c2.overlay.inspect-revision-door")));
        assert_eq!(echo_objects(SceneId("c2-overlay")).len(), 4);
        assert!(state.toggle_echo_layer());
        assert_eq!(state.echo_layer, EchoLayer::Revision2013);
        assert!(!state
            .available_actions()
            .iter()
            .any(|action| action.id == ChoiceId("c2.overlay.inspect-physical-door")));
        assert!(state
            .available_actions()
            .iter()
            .any(|action| action.id == ChoiceId("c2.overlay.inspect-revision-door")));
        for _ in 0..8 {
            assert!(state.toggle_echo_layer());
        }
        assert_eq!(state.available_actions().len(), 1);
        assert_eq!(state.echo_layer, EchoLayer::Revision2013);
        let saved = encode_save(&state);
        let loaded = decode_save(&saved).unwrap();
        assert_eq!(loaded.echo_layer, state.echo_layer);
        assert_eq!(loaded.available_actions(), state.available_actions());
        state
            .select_choice(ChoiceId("c2.overlay.inspect-revision-door"))
            .unwrap();
        assert!(state.supports_echo_overlay());
        assert_eq!(state.echo_layer, EchoLayer::Revision2013);
    }

    #[test]
    fn chapter_two_actor_agency_and_convergence_remain_structured() {
        let mut copied = chapter_two_path_to_overlay(44).unwrap();
        copied
            .select_choice(ChoiceId("c2.overlay.inspect-physical-door"))
            .unwrap();
        copied
            .select_choice(ChoiceId("c2.disagreement.keep-physical"))
            .unwrap();
        copied
            .select_choice(ChoiceId("c2.personal.open-card"))
            .unwrap();
        copied
            .select_choice(ChoiceId("c2.intervention.follow"))
            .unwrap();
        assert!(copied.flags.get("riley_copied_card"));
        assert!(copied.flags.get("elias_closed_outer_gate"));

        let mut sealed = chapter_two_path_to_overlay(44).unwrap();
        sealed
            .select_choice(ChoiceId("c2.overlay.inspect-physical-door"))
            .unwrap();
        sealed
            .select_choice(ChoiceId("c2.disagreement.keep-physical"))
            .unwrap();
        sealed
            .select_choice(ChoiceId("c2.personal.leave-card"))
            .unwrap();
        sealed
            .select_choice(ChoiceId("c2.intervention.follow"))
            .unwrap();
        assert_ne!(copied.relationship(RILEY), sealed.relationship(RILEY));
        assert_ne!(copied.memories, sealed.memories);
        assert!(sealed.flags.get("elias_closed_outer_gate"));
    }

    #[test]
    fn chapter_two_replay_and_every_stable_scene_save_are_deterministic() {
        let first = chapter_two_path(0xC0DE).unwrap();
        let second = chapter_two_path(0xC0DE).unwrap();
        assert_eq!(first, second);
        let mut completed = first.clone();
        completed
            .select_choice(ChoiceId("c2.turning.keep-channel"))
            .unwrap();
        assert!(completed.chapter_complete);
        assert_eq!(decode_save(&encode_save(&completed)).unwrap(), completed);
        let mut paths = 0;
        explore_chapter_two_saves(chapter_two_entry(0x501).unwrap(), 0, &mut paths);
        assert!(paths >= 12);
    }

    #[test]
    fn version_three_chapter_one_completion_migrates_to_chapter_two_entry() {
        let v3 = b"SILICON_ECHOES_SAVE\n\
version=3\n\
node=chapter.turning-point\n\
chapter_complete=1\n\
seed=7\n\
play_time_ms=9\n\
save_generation=4\n\
visited=chapter.turning-point\n\
visit_count=chapter.turning-point:1\n\
observation=sunset_address\n";
        let mut migrated = decode_save(v3).unwrap();
        assert_eq!(migrated.chapter, 1);
        assert_eq!(migrated.echo_layer, EchoLayer::Physical1993);
        migrated.begin_chapter_two().unwrap();
        assert_eq!(migrated.current_node, StoryNodeId("chapter-two.address"));
        assert_eq!(decode_save(&encode_save(&migrated)).unwrap(), migrated);
    }

    fn leave_signal_ready(state: &mut WorldState) {
        state.enter_hotspot(HotspotId::Clock);
        state.select_choice(ChoiceId("clock.accept-date")).unwrap();
        state.advance_uncontrolled_event().unwrap();
        state.enter_hotspot(HotspotId::Window);
        state
            .select_choice(ChoiceId("window.answer-signal"))
            .unwrap();
    }

    fn signal_ready(choice: ChoiceId) -> WorldState {
        let mut state = WorldState::new_seeded(0x1993_0317);
        leave_signal_ready(&mut state);
        state.select_choice(choice).unwrap();
        state
    }

    fn chapter_path_with_revelation(choice: ChoiceId) -> Result<WorldState, StoryError> {
        let mut state = chapter_path(0x1993_0317)?;
        if choice != ChoiceId("revelation.call-riley") {
            state = WorldState::new_seeded(0x1993_0317);
            state.enter_hotspot(HotspotId::Clock);
            state.select_choice(ChoiceId("clock.accept-date"))?;
            state.advance_uncontrolled_event()?;
            state.enter_hotspot(HotspotId::Window);
            state.select_choice(ChoiceId("window.answer-signal"))?;
            state.select_choice(ChoiceId("signal-listen"))?;
            state.select_choice(ChoiceId("hallway.leave-note"))?;
            state.select_choice(ChoiceId("kitchen.read-newspaper"))?;
            state.select_choice(ChoiceId("landing.leave-card"))?;
            state.select_choice(ChoiceId("stairwell.take-stairs"))?;
            state.select_choice(ChoiceId("street.ask-vendor"))?;
            state.select_choice(ChoiceId("diner.test-riley"))?;
            state.select_choice(ChoiceId("phone.hang-up"))?;
            state.select_choice(ChoiceId("repair.borrow-manual"))?;
            state.select_choice(ChoiceId("transit.walk"))?;
            state.advance_uncontrolled_event()?;
            state.select_choice(ChoiceId("archive.ask-public"))?;
            state.select_choice(ChoiceId("stacks.search-terminal"))?;
            state.select_choice(choice)?;
        }
        Ok(state)
    }

    fn chapter_two_entry(seed: u32) -> Result<WorldState, StoryError> {
        let mut state = chapter_path(seed)?;
        state.select_choice(ChoiceId("turning-point.keep-address"))?;
        state.begin_chapter_two()?;
        Ok(state)
    }

    fn chapter_two_entry_with(
        diner: ChoiceId,
        phone: ChoiceId,
        repair: ChoiceId,
        transit: ChoiceId,
        revelation: ChoiceId,
    ) -> Result<WorldState, StoryError> {
        let mut state = WorldState::new_seeded(0x1993_0317);
        state.enter_hotspot(HotspotId::Clock);
        state.select_choice(ChoiceId("clock.accept-date"))?;
        state.advance_uncontrolled_event()?;
        state.enter_hotspot(HotspotId::Window);
        state.select_choice(ChoiceId("window.answer-signal"))?;
        state.select_choice(ChoiceId("signal-listen"))?;
        state.select_choice(ChoiceId("hallway.inspect-note"))?;
        state.select_choice(ChoiceId("kitchen.read-newspaper"))?;
        state.select_choice(ChoiceId("landing.take-card"))?;
        state.select_choice(ChoiceId("stairwell.help-vale"))?;
        state.select_choice(ChoiceId("street.follow-pager"))?;
        state.select_choice(diner)?;
        state.select_choice(phone)?;
        state.select_choice(repair)?;
        state.select_choice(transit)?;
        state.advance_uncontrolled_event()?;
        state.select_choice(ChoiceId("archive.use-card"))?;
        state.select_choice(ChoiceId("stacks.read-ledger"))?;
        state.select_choice(revelation)?;
        state.select_choice(ChoiceId("turning-point.keep-address"))?;
        state.begin_chapter_two()?;
        Ok(state)
    }

    fn chapter_two_path_to_overlay(seed: u32) -> Result<WorldState, StoryError> {
        let mut state = chapter_two_entry(seed)?;
        state.select_choice(ChoiceId("c2.address.call-riley"))?;
        state.select_choice(ChoiceId("c2.contact.wait-riley"))?;
        state.select_choice(ChoiceId("c2.frequency.ask-lio"))?;
        state.select_choice(ChoiceId("c2.records.directory"))?;
        state.select_choice(ChoiceId("c2.route.wait-service"))?;
        state.select_choice(ChoiceId("c2.exterior.ask-caretaker"))?;
        state.select_choice(ChoiceId("c2.caretaker.accept-key"))?;
        state.select_choice(ChoiceId("c2.entry.service-door"))?;
        Ok(state)
    }

    fn chapter_two_path(seed: u32) -> Result<WorldState, StoryError> {
        let mut state = chapter_two_path_to_overlay(seed)?;
        state.select_choice(ChoiceId("c2.overlay.inspect-physical-door"))?;
        state.select_choice(ChoiceId("c2.disagreement.keep-physical"))?;
        state.select_choice(ChoiceId("c2.personal.open-card"))?;
        state.select_choice(ChoiceId("c2.intervention.follow"))?;
        state.select_choice(ChoiceId("c2.chamber.read-revisions"))?;
        state.select_choice(ChoiceId("c2.predicted.preserve"))?;
        state.select_choice(ChoiceId("c2.response.disconnect"))?;
        state.select_choice(ChoiceId("c2.consequence.leave"))?;
        state.select_choice(ChoiceId("c2.displacement.take-cartridge"))?;
        Ok(state)
    }

    fn explore_chapter_two_from(state: WorldState, depth: u8, completed_paths: &mut usize) {
        assert!(depth < 24, "Chapter Two branch did not terminate");
        let current = node(state.current_node).expect("reachable node exists");
        let available = state.available_actions();
        assert!(
            !available.is_empty(),
            "{} has no reachable action",
            current.id.0
        );
        for action in available {
            let mut next = state.clone();
            match next.select_choice(action.id).expect("action is valid") {
                Transition::Node(target) => {
                    assert!(node(target).is_some(), "target is implemented");
                    explore_chapter_two_from(next, depth + 1, completed_paths);
                }
                Transition::Ending(CHAPTER_TWO_ENDING) => *completed_paths += 1,
                Transition::Ending(_) => panic!("unexpected Chapter Two ending"),
            }
        }
        if state.supports_echo_overlay() {
            let mut revision = state;
            revision.toggle_echo_layer();
            for action in revision.available_actions() {
                let mut next = revision.clone();
                match next
                    .select_choice(action.id)
                    .expect("revision action is valid")
                {
                    Transition::Node(_) => {
                        explore_chapter_two_from(next, depth + 1, completed_paths)
                    }
                    Transition::Ending(_) => panic!("overlay cannot end Chapter Two"),
                }
            }
        }
    }

    fn explore_chapter_two_saves(state: WorldState, depth: u8, completed_paths: &mut usize) {
        assert!(depth < 24, "Chapter Two save branch did not terminate");
        let saved =
            decode_save(&encode_save(&state)).expect("stable Chapter Two state round trips");
        assert_eq!(saved, state);
        for action in saved.available_actions() {
            let mut next = saved.clone();
            match next
                .select_choice(action.id)
                .expect("exposed action is valid")
            {
                Transition::Node(_) => explore_chapter_two_saves(next, depth + 1, completed_paths),
                Transition::Ending(CHAPTER_TWO_ENDING) => {
                    assert!(next.chapter_complete);
                    assert_eq!(decode_save(&encode_save(&next)).unwrap(), next);
                    *completed_paths += 1;
                }
                Transition::Ending(_) => panic!("unexpected ending"),
            }
        }
        if saved.supports_echo_overlay() {
            let mut revision = saved;
            revision.toggle_echo_layer();
            assert_eq!(decode_save(&encode_save(&revision)).unwrap(), revision);
            for action in revision.available_actions() {
                let mut next = revision.clone();
                next.select_choice(action.id)
                    .expect("revision action is valid");
                explore_chapter_two_saves(next, depth + 1, completed_paths);
            }
        }
    }

    fn explore_chapter_from(state: WorldState, depth: u8, completed_paths: &mut usize) {
        assert!(depth < 24, "chapter branch did not terminate");
        let current = node(state.current_node).expect("reachable node exists");
        if current.uncontrolled_event {
            let mut next = state.clone();
            let transition = next
                .advance_uncontrolled_event()
                .expect("automatic route exists");
            assert!(matches!(transition, Transition::Node(_)));
            explore_chapter_from(next, depth + 1, completed_paths);
            return;
        }
        let available = state.available_actions();
        assert!(
            !available.is_empty(),
            "{} has no reachable action",
            state.current_node.0
        );
        for action in available {
            let mut next = state.clone();
            let transition = next
                .select_choice(action.id)
                .expect("exposed action is valid");
            match transition {
                Transition::Node(target) => {
                    assert!(node(target).is_some(), "{} is implemented", target.0);
                    explore_chapter_from(next, depth + 1, completed_paths);
                }
                Transition::Ending(ending) => {
                    assert_eq!(ending, TEMPORARY_ENDING);
                    *completed_paths += 1;
                }
            }
        }
    }

    fn explore_save_boundaries(state: WorldState, depth: u8, completed_paths: &mut usize) {
        assert!(depth < 24, "chapter branch did not terminate");
        let saved = decode_save(&encode_save(&state)).expect("stable state round trips");
        assert_eq!(saved, state);
        let current = node(state.current_node).expect("reachable node exists");
        if current.uncontrolled_event {
            let mut next = saved;
            next.advance_uncontrolled_event()
                .expect("automatic route exists");
            explore_save_boundaries(next, depth + 1, completed_paths);
            return;
        }
        if state.current_node == START_NODE {
            let hotspot = if state.flags.get("saw_date") {
                HotspotId::Window
            } else {
                HotspotId::Clock
            };
            let mut next = saved;
            next.try_enter_hotspot(hotspot)
                .expect("required bedroom hotspot is available");
            explore_save_boundaries(next, depth + 1, completed_paths);
            return;
        }
        for action in saved.available_actions() {
            let mut next = saved.clone();
            match next
                .select_choice(action.id)
                .expect("exposed action is valid")
            {
                Transition::Node(_) => explore_save_boundaries(next, depth + 1, completed_paths),
                Transition::Ending(TEMPORARY_ENDING) => {
                    assert!(next.chapter_complete);
                    let loaded = decode_save(&encode_save(&next)).expect("ending save round trips");
                    assert_eq!(loaded, next);
                    *completed_paths += 1;
                }
                Transition::Ending(_) => panic!("unexpected ending"),
            }
        }
    }
}
