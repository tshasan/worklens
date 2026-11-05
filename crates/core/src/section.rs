#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Waiting,
    Land,
    Returned,
    Uplift,
    Others,
    Sprint,
    Ideas,
    Drift,
    Other,
    Team,
}

impl Section {
    pub const ALL: [Self; 10] = [
        Self::Waiting,
        Self::Land,
        Self::Returned,
        Self::Uplift,
        Self::Others,
        Self::Sprint,
        Self::Ideas,
        Self::Drift,
        Self::Other,
        Self::Team,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Land => "land",
            Self::Returned => "returned",
            Self::Uplift => "uplift",
            Self::Others => "others",
            Self::Sprint => "sprint",
            Self::Ideas => "ideas",
            Self::Drift => "drift",
            Self::Other => "other",
            Self::Team => "team",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting on you",
            Self::Land => "Ready to land",
            Self::Returned => "Returned to you",
            Self::Uplift => "Needs uplift",
            Self::Others => "Waiting on others",
            Self::Sprint => "Sprint",
            Self::Ideas => "Ideas",
            Self::Drift => "Drift",
            Self::Other => "Everything else",
            Self::Team => "Team drift",
        }
    }

    pub const fn short(self) -> &'static str {
        match self {
            Self::Waiting => "You",
            Self::Land => "Land",
            Self::Returned => "Returned",
            Self::Uplift => "Uplift",
            Self::Others => "Others",
            Self::Sprint => "Sprint",
            Self::Ideas => "Ideas",
            Self::Drift => "Drift",
            Self::Other => "Other",
            Self::Team => "Team",
        }
    }

    pub const fn empty(self) -> &'static str {
        match self {
            Self::Waiting => "Nothing waiting on you.",
            Self::Land => "None of your revisions are accepted and waiting to land.",
            Self::Returned => "No changes requested on your revisions.",
            Self::Uplift => "None of your fixes are waiting to go to Beta or ESR.",
            Self::Others => "None of your revisions are waiting for review.",
            Self::Sprint => "No tickets of yours in the active sprint.",
            Self::Ideas => "None of your tickets roll up to a Jira Product Discovery idea.",
            Self::Drift => "Your bugs and their Jira tickets agree.",
            Self::Other => "Nothing else assigned to you.",
            Self::Team => "Every team bug agrees with its Jira ticket.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_display_order() {
        for (i, s) in Section::ALL.into_iter().enumerate() {
            assert_eq!(s.index(), i);
        }
    }
}
