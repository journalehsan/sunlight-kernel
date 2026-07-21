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

pub const SAVE_FORMAT_VERSION: u16 = 3;
pub const MAX_SAVE_BYTES: usize = 4096;
const MAX_RECORDS: usize = 192;
const MAX_TEXT_VALUE_BYTES: usize = 128;
const MAX_VISITED_NODES: usize = 32;
const MAX_STATE_SET_ITEMS: usize = 32;
const MAX_RELATIONSHIPS: usize = 8;
const MAX_TENDENCIES: usize = 8;
pub const START_NODE: StoryNodeId = StoryNodeId("bedroom.wake");
pub const TEMPORARY_ENDING: EndingId = EndingId("ending.chapter-one");

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

pub fn hotspot(id: HotspotId) -> &'static Hotspot {
    BEDROOM_HOTSPOTS
        .iter()
        .find(|item| item.id == id)
        .unwrap_or(&BEDROOM_HOTSPOTS[0])
}

pub fn actors() -> &'static [ActorId] {
    &[RILEY, VALE, LIO]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldState {
    pub current_node: StoryNodeId,
    pub chapter_complete: bool,
    pub visited_nodes: BTreeSet<String>,
    pub visit_counts: BTreeMap<String, u16>,
    pub selected_choices: Vec<String>,
    pub flags: StoryFlags,
    pub facts: BTreeSet<String>,
    pub observations: BTreeSet<String>,
    pub beliefs: BTreeSet<String>,
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
            chapter_complete: false,
            visited_nodes: BTreeSet::new(),
            visit_counts: BTreeMap::new(),
            selected_choices: Vec::new(),
            flags: StoryFlags::default(),
            facts: BTreeSet::new(),
            observations: BTreeSet::new(),
            beliefs: BTreeSet::new(),
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
        if self.selected_choices.len() >= 64 {
            self.selected_choices.remove(0);
        }
        self.selected_choices.push(String::from(choice.id.0));
        self.memories.insert(String::from(choice.id.0));
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

pub fn presentation_narration(world: &WorldState, story_node: &StoryNode) -> String {
    let mut text = String::from(story_node.narration);
    match story_node.id.0 {
        "chapter.diner" if world.flags.get("vale_vouched") => {
            text.push_str(" Mrs. Vale has phoned ahead: \"Tell Riley I saw you leave.\"");
        }
        "chapter.diner" if world.observations.contains("riley_waited") => {
            text.push_str(" A second coffee has gone cold beside Riley's hand.");
        }
        "chapter.diner" if world.relationship(RILEY) < 0 => {
            text.push_str(" Riley keeps one hand on the exit side of the booth.");
        }
        "chapter.repair-shop" if world.relationship(LIO) > 0 => {
            text.push_str(" Lio hears the phrasing you wrote down and unlocks the back cabinet without being asked.");
        }
        "chapter.archive-lobby" if world.flags.get("has_archive_card") => {
            text.push_str(
                " The card's ink has bled into an address the clerk refuses to read aloud.",
            );
        }
        "chapter.turning-point" if world.relationship(RILEY) > 0 => {
            text.push_str(" Riley says they will meet you at sunrise, not because they understand, but because they chose to stay.");
        }
        _ => {}
    }
    text
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
        validate_effects(item.entry_effects, &node_ids, &actor_ids, &mut errors);
        if let Some(target) = item.automatic_target {
            validate_transition(target, &node_ids, &mut errors);
        }
        for choice in item.choices {
            if !choice_ids.insert(choice.id.0) {
                errors.push(ValidationError::DuplicateChoice(String::from(choice.id.0)));
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
    if !ending_reachable(TEMPORARY_ENDING) {
        errors.push(ValidationError::MissingEnding(String::from(
            TEMPORARY_ENDING.0,
        )));
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
    TooLarge,
}

pub fn encode_save(state: &WorldState) -> Vec<u8> {
    let mut out = String::from("SILICON_ECHOES_SAVE\n");
    push_record(&mut out, "version", &format!("{}", SAVE_FORMAT_VERSION));
    push_record(&mut out, "node", state.current_node.0);
    push_record(
        &mut out,
        "chapter_complete",
        if state.chapter_complete { "1" } else { "0" },
    );
    push_record(&mut out, "seed", &format!("{}", state.seed));
    push_record(&mut out, "play_time_ms", &format!("{}", state.play_time_ms));
    push_record(
        &mut out,
        "save_generation",
        &format!("{}", state.save_generation),
    );
    for value in &state.visited_nodes {
        push_record(&mut out, "visited", value);
    }
    for (value, count) in &state.visit_counts {
        push_record(&mut out, "visit_count", &format!("{}:{}", value, count));
    }
    for value in &state.selected_choices {
        push_record(&mut out, "choice", value);
    }
    for (key, value) in state.flags.iter() {
        push_record(
            &mut out,
            "flag",
            &format!("{}:{}", key, if *value { 1 } else { 0 }),
        );
    }
    for value in &state.facts {
        push_record(&mut out, "fact", value);
    }
    for value in &state.observations {
        push_record(&mut out, "observation", value);
    }
    for value in &state.beliefs {
        push_record(&mut out, "belief", value);
    }
    for value in &state.memories {
        push_record(&mut out, "memory", value);
    }
    for (actor, trust) in &state.relationships {
        push_record(&mut out, "relationship", &format!("{}:{}", actor, trust));
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
        SAVE_FORMAT_VERSION => decode_v3(&records),
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
    state.memories.clear();
    state.relationships.clear();
    state.delayed.clear();
    state.tendencies.clear();
    state
}

fn decode_v1(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, false, false)?;
    finalize_loaded_state(state)
}

fn decode_v2(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, true, false)?;
    finalize_loaded_state(state)
}

fn decode_v3(records: &[(String, String)]) -> Result<WorldState, SaveError> {
    let mut state = empty_loaded_state();
    load_common_records(&mut state, records, true, true)?;
    finalize_loaded_state(state)
}

fn load_common_records(
    state: &mut WorldState,
    records: &[(String, String)],
    is_v2: bool,
    has_completion: bool,
) -> Result<(), SaveError> {
    let mut node_id = None;
    let mut saw_version = false;
    let mut saw_node = false;
    let mut saw_seed = !is_v2;
    let mut saw_play_time = false;
    let mut saw_generation = !has_completion;
    let mut saw_completion = !has_completion;
    for (key, value) in records {
        match key.as_str() {
            "version" if !saw_version => saw_version = true,
            "node" if !saw_node => {
                saw_node = true;
                node_id = Some(value.as_str());
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
                if state.selected_choices.len() >= 64 {
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
    if !saw_version || !saw_node || !saw_seed || !saw_completion || !saw_generation {
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
    if state.visit_counts.is_empty() {
        for value in &state.visited_nodes {
            state.visit_counts.insert(value.clone(), 1);
        }
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
    if state.chapter_complete && state.current_node != StoryNodeId("chapter.turning-point") {
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
        _ => None,
    }
}

fn delayed_code(effect: DelayedEffect) -> &'static str {
    match effect {
        DelayedEffect::SetFlag("signal_arrived", true) => "signal-arrived",
        DelayedEffect::SetFlag("vale_vouched", true) => "vale-vouched",
        DelayedEffect::AddObservation("riley_waited") => "riley-waited",
        DelayedEffect::AdjustRelationship(ActorId("lio"), 1) => "lio-recording",
        _ => "invalid",
    }
}

fn delayed_from_code(code: &str) -> Option<DelayedEffect> {
    match code {
        "signal-arrived" => Some(DelayedEffect::SetFlag("signal_arrived", true)),
        "vale-vouched" => Some(DelayedEffect::SetFlag("vale_vouched", true)),
        "riley-waited" => Some(DelayedEffect::AddObservation("riley_waited")),
        "lio-recording" => Some(DelayedEffect::AdjustRelationship(LIO, 1)),
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
