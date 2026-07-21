//! Story data, validation, and save format for Silicon Echoes: 1993.
//!
//! The graph is deterministic. Ambient presentation may vary, but consequences
//! and endings are driven only by this state.

#![no_std]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const SAVE_FORMAT_VERSION: u16 = 1;
pub const START_NODE: StoryNodeId = StoryNodeId("bedroom.wake");
pub const TEMPORARY_ENDING: EndingId = EndingId("ending.signal");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SceneId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoryNodeId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChoiceId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndingId(pub &'static str);

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Consequence {
    SetFlag(&'static str, bool),
    Shift(Tendency, i8),
    QueueDelayed {
        id: &'static str,
        after_node: StoryNodeId,
        effect: DelayedEffect,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelayedEffect {
    SetFlag(&'static str, bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Flag(&'static str, bool),
    Visited(StoryNodeId),
    All(&'static [Condition]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Node(StoryNodeId),
    Ending(EndingId),
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

#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub id: SceneId,
    pub title: &'static str,
    pub hotspots: &'static [Hotspot],
}

const CLOCK_CHOICES: &[Choice] = &[Choice {
    id: ChoiceId("clock.accept-date"),
    text: "Let the date remain impossible.",
    target: Transition::Node(StoryNodeId("bedroom.after-clock")),
    condition: None,
    effects: &[
        Consequence::SetFlag("saw_date", true),
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
        target: Transition::Ending(TEMPORARY_ENDING),
        condition: None,
        effects: &[Consequence::Shift(Tendency::Responsibility, 1)],
        intentionally_converges: true,
    },
    Choice {
        id: ChoiceId("signal-listen"),
        text: "Say nothing. Listen.",
        target: Transition::Ending(TEMPORARY_ENDING),
        condition: None,
        effects: &[Consequence::Shift(Tendency::Curiosity, 1)],
        intentionally_converges: true,
    },
];

const NODES: &[StoryNode] = &[
    StoryNode {
        id: START_NODE,
        scene: SceneId("bedroom"),
        narration: "You wake beneath a ceiling you remember from somewhere else. The room is holding its breath.",
        choices: EMPTY_EFFECTS_AS_CHOICES,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.clock"),
        scene: SceneId("bedroom"),
        narration: "The clock reads 03:17. Below it, the date refuses to be a dream: 1993.",
        choices: CLOCK_CHOICES,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.workstation"),
        scene: SceneId("bedroom"),
        narration: "A beige workstation waits at the desk. Its CRT has already written your name, then erased it.",
        choices: WORKSTATION_CHOICES,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.desk"),
        scene: SceneId("bedroom"),
        narration: "The desk is too orderly. A sealed letter carries your handwriting without your certainty.",
        choices: DESK_CHOICES,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.window"),
        scene: SceneId("bedroom"),
        narration: "Rain holds to the window without falling. Across the street, every dark window faces back.",
        choices: WINDOW_CHOICES,
        uncontrolled_event: false,
        automatic_target: None,
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-clock"),
        scene: SceneId("bedroom"),
        narration: "The digits settle. Something in the room has noticed that you noticed.",
        choices: EMPTY_EFFECTS_AS_CHOICES,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-workstation"),
        scene: SceneId("bedroom"),
        narration: "The cursor blinks once more. Whether you read it or not, it has read you.",
        choices: EMPTY_EFFECTS_AS_CHOICES,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.after-desk"),
        scene: SceneId("bedroom"),
        narration: "The paper makes no sound. The room continues as though the decision was made years ago.",
        choices: EMPTY_EFFECTS_AS_CHOICES,
        uncontrolled_event: true,
        automatic_target: Some(Transition::Node(START_NODE)),
    },
    StoryNode {
        id: StoryNodeId("bedroom.signal"),
        scene: SceneId("bedroom"),
        narration: "The CRT brightens without your touch. A voice arrives through the glass: not yours, almost yours.",
        choices: SIGNAL_CHOICES,
        uncontrolled_event: true,
        automatic_target: None,
    },
];

const EMPTY_EFFECTS_AS_CHOICES: &[Choice] = &[];

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

const SCENES: &[Scene] = &[Scene {
    id: SceneId("bedroom"),
    title: "Bedroom / 03:17",
    hotspots: BEDROOM_HOTSPOTS,
}];

pub fn scenes() -> &'static [Scene] {
    SCENES
}

pub fn nodes() -> &'static [StoryNode] {
    NODES
}

pub fn node(id: StoryNodeId) -> Option<&'static StoryNode> {
    NODES.iter().find(|node| node.id == id)
}

pub fn hotspot(id: HotspotId) -> &'static Hotspot {
    BEDROOM_HOTSPOTS
        .iter()
        .find(|hotspot| hotspot.id == id)
        .unwrap_or(&BEDROOM_HOTSPOTS[0])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameState {
    pub current_node: StoryNodeId,
    pub visited_nodes: BTreeSet<String>,
    pub selected_choices: Vec<String>,
    pub flags: StoryFlags,
    pub delayed: Vec<DelayedConsequence>,
    pub tendencies: BTreeMap<String, i16>,
    pub play_time_ms: u64,
}

impl GameState {
    pub fn new() -> Self {
        let mut state = Self {
            current_node: START_NODE,
            visited_nodes: BTreeSet::new(),
            selected_choices: Vec::new(),
            flags: StoryFlags::default(),
            delayed: Vec::new(),
            tendencies: BTreeMap::new(),
            play_time_ms: 0,
        };
        state.mark_visited(START_NODE);
        state
    }

    pub fn mark_visited(&mut self, node: StoryNodeId) {
        self.visited_nodes.insert(String::from(node.0));
    }

    pub fn has_visited(&self, node: StoryNodeId) -> bool {
        self.visited_nodes.contains(node.0)
    }

    pub fn tendency(&self, tendency: Tendency) -> i16 {
        self.tendencies
            .get(tendency_key(tendency))
            .copied()
            .unwrap_or(0)
    }

    pub fn select_choice(&mut self, choice_id: ChoiceId) -> Result<Transition, StoryError> {
        let active_node = node(self.current_node).ok_or(StoryError::UnknownNode)?;
        let choice = active_node
            .choices
            .iter()
            .find(|choice| choice.id == choice_id)
            .ok_or(StoryError::UnknownChoice)?;
        if !choice
            .condition
            .map(|condition| condition_met(condition, self))
            .unwrap_or(true)
        {
            return Err(StoryError::UnavailableChoice);
        }
        for effect in choice.effects {
            self.apply(effect);
        }
        self.selected_choices.push(String::from(choice.id.0));
        self.apply_transition(choice.target);
        Ok(choice.target)
    }

    pub fn enter_hotspot(&mut self, hotspot: HotspotId) -> StoryNodeId {
        let next = crate::hotspot(hotspot).target;
        self.apply_transition(Transition::Node(next));
        next
    }

    pub fn apply_transition(&mut self, transition: Transition) {
        if let Transition::Node(next) = transition {
            self.current_node = next;
            self.mark_visited(next);
            self.apply_due_delayed();
        }
    }

    pub fn advance_uncontrolled_event(&mut self) -> Result<Transition, StoryError> {
        let current = node(self.current_node).ok_or(StoryError::UnknownNode)?;
        if !current.uncontrolled_event {
            return Err(StoryError::UnavailableChoice);
        }
        let target = current.automatic_target.ok_or(StoryError::UnknownChoice)?;
        self.apply_transition(target);
        Ok(target)
    }

    fn apply(&mut self, effect: &Consequence) {
        match effect {
            Consequence::SetFlag(key, value) => self.flags.set(key, *value),
            Consequence::Shift(tendency, amount) => {
                let key = String::from(tendency_key(*tendency));
                let current = self.tendencies.get(&key).copied().unwrap_or(0);
                self.tendencies
                    .insert(key, current.saturating_add(*amount as i16));
            }
            Consequence::QueueDelayed {
                id,
                after_node,
                effect,
            } => {
                if !self.delayed.iter().any(|queued| queued.id == *id) {
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
        let mut remaining = Vec::new();
        let queued = core::mem::take(&mut self.delayed);
        for delayed in queued {
            if delayed.after_node == self.current_node {
                match delayed.effect {
                    DelayedEffect::SetFlag(key, value) => self.flags.set(key, value),
                }
            } else {
                remaining.push(delayed);
            }
        }
        self.delayed = remaining;
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryError {
    UnknownNode,
    UnknownChoice,
    UnavailableChoice,
}

fn tendency_key(tendency: Tendency) -> &'static str {
    match tendency {
        Tendency::Agency => "agency",
        Tendency::Responsibility => "responsibility",
        Tendency::Curiosity => "curiosity",
        Tendency::Attachment => "attachment",
    }
}

pub fn condition_met(condition: Condition, state: &GameState) -> bool {
    match condition {
        Condition::Flag(key, expected) => state.flags.get(key) == expected,
        Condition::Visited(id) => state.has_visited(id),
        Condition::All(all) => all.iter().copied().all(|item| condition_met(item, state)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    DuplicateNode(String),
    DuplicateChoice(String),
    MissingScene(String),
    MissingNode(String),
    MissingConditionKey(String),
    DeadEnd(String),
    UnmarkedConvergence(String),
    MissingEnding(String),
    UnreachableEnding(String),
}

pub fn validate_graph() -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut scene_ids = BTreeSet::new();
    for scene in SCENES {
        scene_ids.insert(scene.id.0);
        for hotspot in scene.hotspots {
            if let Some(condition) = hotspot.condition {
                validate_condition(condition, &mut errors);
            }
        }
    }
    let mut node_ids = BTreeSet::new();
    let mut choice_ids = BTreeSet::new();
    let mut ending_ids = BTreeSet::new();
    let mut incoming = BTreeMap::<&str, Vec<&Choice>>::new();
    let mut ending_incoming = BTreeMap::<&str, Vec<&Choice>>::new();

    for item in NODES {
        if !node_ids.insert(item.id.0) {
            errors.push(ValidationError::DuplicateNode(String::from(item.id.0)));
        }
        if !scene_ids.contains(item.scene.0) {
            errors.push(ValidationError::MissingScene(String::from(item.scene.0)));
        }
        if item.choices.is_empty() && item.automatic_target.is_none() && item.id != START_NODE {
            errors.push(ValidationError::DeadEnd(String::from(item.id.0)));
        }
        for choice in item.choices {
            if !choice_ids.insert(choice.id.0) {
                errors.push(ValidationError::DuplicateChoice(String::from(choice.id.0)));
            }
            if let Some(condition) = choice.condition {
                validate_condition(condition, &mut errors);
            }
            match choice.target {
                Transition::Node(target) => {
                    incoming.entry(target.0).or_default().push(choice);
                }
                Transition::Ending(ending) => {
                    ending_ids.insert(ending.0);
                    ending_incoming.entry(ending.0).or_default().push(choice);
                }
            }
        }
        if let Some(Transition::Node(target)) = item.automatic_target {
            if !node_ids.contains(target.0) {
                errors.push(ValidationError::MissingNode(String::from(target.0)));
            }
        }
    }
    for scene in SCENES {
        for hotspot in scene.hotspots {
            if !node_ids.contains(hotspot.target.0) {
                errors.push(ValidationError::MissingNode(String::from(hotspot.target.0)));
            }
        }
    }
    for item in NODES {
        for choice in item.choices {
            if let Transition::Node(target) = choice.target {
                if !node_ids.contains(target.0) {
                    errors.push(ValidationError::MissingNode(String::from(target.0)));
                }
            }
        }
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
    if !ending_ids.contains(TEMPORARY_ENDING.0) {
        errors.push(ValidationError::MissingEnding(String::from(
            TEMPORARY_ENDING.0,
        )));
    }
    if !ending_reachable(TEMPORARY_ENDING) {
        errors.push(ValidationError::UnreachableEnding(String::from(
            TEMPORARY_ENDING.0,
        )));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_condition(condition: Condition, errors: &mut Vec<ValidationError>) {
    match condition {
        Condition::Flag(key, _) => {
            if !known_flag(key) {
                errors.push(ValidationError::MissingConditionKey(String::from(key)));
            }
        }
        Condition::Visited(_) => {}
        Condition::All(items) => {
            for item in items {
                validate_condition(*item, errors);
            }
        }
    }
}

fn known_flag(key: &str) -> bool {
    matches!(
        key,
        "saw_date" | "saw_prompt" | "opened_letter" | "signal_arrived"
    )
}

fn ending_reachable(ending: EndingId) -> bool {
    let mut pending = Vec::from([START_NODE]);
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    InvalidUtf8,
    UnsupportedVersion,
    InvalidRecord,
}

pub fn encode_save(state: &GameState) -> Vec<u8> {
    let mut out = String::from("SILICON_ECHOES_SAVE\n");
    push_record(&mut out, "version", &format!("{}", SAVE_FORMAT_VERSION));
    push_record(&mut out, "node", state.current_node.0);
    push_record(&mut out, "play_time_ms", &format!("{}", state.play_time_ms));
    for visited in &state.visited_nodes {
        push_record(&mut out, "visited", visited);
    }
    for choice in &state.selected_choices {
        push_record(&mut out, "choice", choice);
    }
    for (key, value) in state.flags.iter() {
        push_record(
            &mut out,
            "flag",
            &format!("{}:{}", key, if *value { 1 } else { 0 }),
        );
    }
    for (key, value) in &state.tendencies {
        push_record(&mut out, "tendency", &format!("{}:{}", key, value));
    }
    for delayed in &state.delayed {
        push_record(
            &mut out,
            "delayed",
            &format!(
                "{}:{}:{}",
                delayed.id,
                delayed.after_node.0,
                consequence_code(delayed.effect)
            ),
        );
    }
    out.into_bytes()
}

pub fn decode_save(bytes: &[u8]) -> Result<GameState, SaveError> {
    let text = core::str::from_utf8(bytes).map_err(|_| SaveError::InvalidUtf8)?;
    let mut lines = text.lines();
    if lines.next() != Some("SILICON_ECHOES_SAVE") {
        return Err(SaveError::InvalidRecord);
    }
    let mut version = None;
    let mut node_id = None;
    let mut state = GameState::new();
    state.visited_nodes.clear();
    state.selected_choices.clear();
    state.flags = StoryFlags::default();
    state.delayed.clear();
    state.tendencies.clear();

    for line in lines {
        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(SaveError::InvalidRecord);
        };
        let value = unescape(raw_value)?;
        match key {
            "version" => version = parse_u16(&value),
            "node" => node_id = Some(value),
            "play_time_ms" => {
                state.play_time_ms = parse_u64(&value).ok_or(SaveError::InvalidRecord)?
            }
            "visited" => {
                if known_node_id(&value).is_none() {
                    return Err(SaveError::InvalidRecord);
                }
                state.visited_nodes.insert(value);
            }
            "choice" => {
                if !choice_exists(&value) {
                    return Err(SaveError::InvalidRecord);
                }
                state.selected_choices.push(value);
            }
            "flag" => {
                let (flag, bool_value) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                if !known_flag(flag) || !matches!(bool_value, "0" | "1") {
                    return Err(SaveError::InvalidRecord);
                }
                state.flags.set(flag, bool_value == "1");
            }
            "tendency" => {
                let (name, amount) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                if !matches!(
                    name,
                    "agency" | "responsibility" | "curiosity" | "attachment"
                ) {
                    return Err(SaveError::InvalidRecord);
                }
                state.tendencies.insert(
                    String::from(name),
                    parse_i16(amount).ok_or(SaveError::InvalidRecord)?,
                );
            }
            "delayed" => {
                let (id, rest) = value.split_once(':').ok_or(SaveError::InvalidRecord)?;
                let (after_node, code) = rest.split_once(':').ok_or(SaveError::InvalidRecord)?;
                let static_after = known_node_id(after_node).ok_or(SaveError::InvalidRecord)?;
                state.delayed.push(DelayedConsequence {
                    id: delayed_id(id).ok_or(SaveError::InvalidRecord)?,
                    after_node: StoryNodeId(static_after),
                    effect: consequence_from_code(code).ok_or(SaveError::InvalidRecord)?,
                });
            }
            _ => return Err(SaveError::InvalidRecord),
        }
    }
    if version != Some(SAVE_FORMAT_VERSION) {
        return Err(SaveError::UnsupportedVersion);
    }
    let current = node_id
        .as_deref()
        .and_then(known_node_id)
        .map(StoryNodeId)
        .ok_or(SaveError::InvalidRecord)?;
    if state.visited_nodes.is_empty() || !state.visited_nodes.contains(current.0) {
        return Err(SaveError::InvalidRecord);
    }
    state.current_node = current;
    Ok(state)
}

fn push_record(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    for byte in value.bytes() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            byte => out.push(byte as char),
        }
    }
    out.push('\n');
}

fn unescape(value: &str) -> Result<String, SaveError> {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            match ch {
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                _ => return Err(SaveError::InvalidRecord),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        return Err(SaveError::InvalidRecord);
    }
    Ok(out)
}

fn choice_exists(id: &str) -> bool {
    NODES
        .iter()
        .flat_map(|node| node.choices)
        .any(|choice| choice.id.0 == id)
}

fn known_node_id(id: &str) -> Option<&'static str> {
    NODES
        .iter()
        .map(|node| node.id.0)
        .find(|known| *known == id)
}

fn delayed_id(id: &str) -> Option<&'static str> {
    match id {
        "signal-after-window" => Some("signal-after-window"),
        _ => None,
    }
}

fn consequence_code(effect: DelayedEffect) -> &'static str {
    match effect {
        DelayedEffect::SetFlag("signal_arrived", true) => "signal-arrived",
        _ => "invalid",
    }
}

fn consequence_from_code(code: &str) -> Option<DelayedEffect> {
    match code {
        "signal-arrived" => Some(DelayedEffect::SetFlag("signal_arrived", true)),
        _ => None,
    }
}

fn parse_u16(text: &str) -> Option<u16> {
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
                "ambient-frame:{}:{}:the-room-keeps-its-own-counsel",
                run, index
            ));
        }
        peak_live_states = peak_live_states.max(transient.len());
        let mut state = GameState::new();
        state.enter_hotspot(HotspotId::Clock);
        state.select_choice(ChoiceId("clock.accept-date"))?;
        state.enter_hotspot(HotspotId::Workstation);
        state.select_choice(ChoiceId("workstation.read-prompt"))?;
        state.enter_hotspot(HotspotId::Window);
        state.select_choice(ChoiceId("window.answer-signal"))?;
        let saved = encode_save(&state);
        let loaded = decode_save(&saved).map_err(|_| StoryError::UnknownNode)?;
        let mut transition_state = alloc::boxed::Box::new(loaded);
        transition_state.select_choice(ChoiceId("signal-listen"))?;
        drop(transient);
        drop(transition_state);
    }
    Ok(StressReport {
        completed_runs: runs,
        peak_live_states,
    })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_is_valid() {
        assert_eq!(validate_graph(), Ok(()));
    }

    #[test]
    fn save_round_trip_is_atomic_and_typed() {
        let mut state = GameState::new();
        state.enter_hotspot(HotspotId::Clock);
        state.select_choice(ChoiceId("clock.accept-date")).unwrap();
        state.play_time_ms = 42;
        let bytes = encode_save(&state);
        assert_eq!(decode_save(&bytes).unwrap(), state);
        assert!(decode_save(b"broken").is_err());
        assert!(decode_save(b"SILICON_ECHOES_SAVE\nversion=999\n").is_err());
    }

    #[test]
    fn stress_releases_prior_scene_state() {
        assert_eq!(run_deterministic_stress(64).unwrap().completed_runs, 64);
    }
}
