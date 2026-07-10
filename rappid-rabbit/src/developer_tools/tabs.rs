#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeveloperToolTab {
    Console,
    DomInspector,
    Network,
}

impl DeveloperToolTab {
    pub const ALL: [Self; 3] = [Self::Console, Self::DomInspector, Self::Network];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::DomInspector => "DOM Inspector",
            Self::Network => "Network",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Console => 0,
            Self::DomInspector => 1,
            Self::Network => 2,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Console),
            1 => Some(Self::DomInspector),
            2 => Some(Self::Network),
            _ => None,
        }
    }
}
