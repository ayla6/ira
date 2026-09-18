//! The dimension the library's games are categorized by — the sidebar
//! derives one collapsible section per value and the grid filters to
//! the selected one.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    #[default]
    Off,
    Developer,
    Publisher,
    Genre,
    Family,
    Console,
    Year,
}

impl GroupBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupBy::Off => "off",
            GroupBy::Developer => "developer",
            GroupBy::Publisher => "publisher",
            GroupBy::Genre => "genre",
            GroupBy::Family => "family",
            GroupBy::Console => "console",
            GroupBy::Year => "year",
        }
    }

    pub fn parse_group_by(s: &str) -> Self {
        match s {
            "developer" => GroupBy::Developer,
            "publisher" => GroupBy::Publisher,
            "genre" => GroupBy::Genre,
            "family" => GroupBy::Family,
            "console" => GroupBy::Console,
            "year" => GroupBy::Year,
            _ => GroupBy::Off,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            GroupBy::Off => "None",
            GroupBy::Developer => "Developer",
            GroupBy::Publisher => "Publisher",
            GroupBy::Genre => "Genre",
            GroupBy::Family => "Family",
            GroupBy::Console => "Console",
            GroupBy::Year => "Release year",
        }
    }

    /// Every choice, in menu order.
    pub const ALL: &[GroupBy] = &[
        GroupBy::Off,
        GroupBy::Developer,
        GroupBy::Publisher,
        GroupBy::Genre,
        GroupBy::Family,
        GroupBy::Console,
        GroupBy::Year,
    ];

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|g| g == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

impl serde::Serialize for GroupBy {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for GroupBy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(GroupBy::parse_group_by(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::GroupBy;

    #[test]
    fn test_group_by_round_trips_and_defaults() {
        for mode in GroupBy::ALL {
            assert_eq!(*mode, GroupBy::parse_group_by(mode.as_str()));
        }
        assert_eq!(GroupBy::parse_group_by("nonsense"), GroupBy::Off);
        assert_eq!(GroupBy::default(), GroupBy::Off);
        assert_eq!(GroupBy::from_index(3), GroupBy::Genre);
        assert_eq!(GroupBy::from_index(4), GroupBy::Family);
        assert_eq!(GroupBy::from_index(99), GroupBy::Off);
    }
}
