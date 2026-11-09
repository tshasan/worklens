#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Waiting,
    Land,
    Returned,
    Others,
    Sprint,
    Ideas,
    Drift,
    Other,
    Team,
    Landed,
    Backlog,
    Local,
}

impl Section {
    pub const ALL: [Self; 12] = [
        Self::Waiting,
        Self::Land,
        Self::Returned,
        Self::Others,
        Self::Sprint,
        Self::Ideas,
        Self::Drift,
        Self::Other,
        Self::Team,
        Self::Landed,
        Self::Backlog,
        Self::Local,
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
            Self::Others => "others",
            Self::Sprint => "sprint",
            Self::Ideas => "ideas",
            Self::Drift => "drift",
            Self::Other => "other",
            Self::Team => "team",
            Self::Landed => "landed",
            Self::Backlog => "backlog",
            Self::Local => "local",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting on you",
            Self::Land => "Ready to land",
            Self::Returned => "Returned to you",
            Self::Others => "Waiting on others",
            Self::Sprint => "Sprint",
            Self::Ideas => "Ideas",
            Self::Drift => "Drift",
            Self::Other => "Everything else",
            Self::Team => "Team drift",
            Self::Landed => "Landed",
            Self::Backlog => "Backlog",
            Self::Local => "Not uploaded",
        }
    }

    pub const fn short(self) -> &'static str {
        match self {
            Self::Waiting => "You",
            Self::Land => "Land",
            Self::Returned => "Returned",
            Self::Others => "Others",
            Self::Sprint => "Sprint",
            Self::Ideas => "Ideas",
            Self::Drift => "Drift",
            Self::Other => "Other",
            Self::Team => "Team",
            Self::Landed => "Landed",
            Self::Backlog => "Backlog",
            Self::Local => "Local",
        }
    }

    pub const fn empty(self) -> &'static str {
        match self {
            Self::Waiting => "Nothing waiting on you.",
            Self::Land => "None of your revisions are accepted and waiting to land.",
            Self::Returned => "No changes requested on your revisions.",
            Self::Others => "None of your revisions are waiting for review.",
            Self::Sprint => "No tickets of yours in the active sprint.",
            Self::Ideas => "None of your tickets roll up to a Jira Product Discovery idea.",
            Self::Drift => "Your bugs and their Jira tickets agree.",
            Self::Other => "Nothing else assigned to you.",
            Self::Team => "Every team bug agrees with its Jira ticket.",
            Self::Landed => "No recent fixes or observed landings in this cache.",
            Self::Backlog => "Nothing assigned to you is waiting to be started.",
            Self::Local => "Every local commit of yours is on Phabricator.",
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
