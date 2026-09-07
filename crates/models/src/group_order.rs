#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupOrder {
    #[default]
    Alphabetical,
    Size,
}

impl GroupOrder {
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupOrder::Alphabetical => "alphabetical",
            GroupOrder::Size => "size",
        }
    }

    pub fn parse_group_order(s: &str) -> Self {
        match s {
            "size" => GroupOrder::Size,
            _ => GroupOrder::Alphabetical,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            GroupOrder::Alphabetical => "Alphabetical",
            GroupOrder::Size => "Most Games",
        }
    }

    pub const ALL: &[GroupOrder] = &[GroupOrder::Alphabetical, GroupOrder::Size];
}

impl serde::Serialize for GroupOrder {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for GroupOrder {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(GroupOrder::parse_group_order(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_order_roundtrip() {
        for order in GroupOrder::ALL {
            assert_eq!(GroupOrder::parse_group_order(order.as_str()), *order);
        }
    }

    #[test]
    fn test_group_order_unknown_defaults_alphabetical() {
        assert_eq!(
            GroupOrder::parse_group_order("garbage"),
            GroupOrder::Alphabetical
        );
    }
}
