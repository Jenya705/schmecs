pub mod unit;

#[derive(Clone, Copy)]
pub enum AccessConflict {
    Yes,
    No,
}

impl AccessConflict {
    pub fn yes_if_true(val: bool) -> Self {
        match val {
            true => Self::Yes,
            false => Self::No,
        }
    }

    pub fn no_if_true(val: bool) -> Self {
        Self::yes_if_true(!val)
    }

    pub fn yes(self) -> bool {
        matches!(self, Self::Yes)
    }

    pub fn no(self) -> bool {
        !self.yes()
    }

    pub fn then(self, map: impl FnOnce() -> Self) -> Self {
        match self {
            AccessConflict::Yes => AccessConflict::Yes,
            AccessConflict::No => map(),
        }
    }
}
