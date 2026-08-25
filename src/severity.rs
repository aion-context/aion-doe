//! Change severity. Its job is to keep a substantive amendment to a
//! regulation from being buried under an accreditation-roster refresh.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    None,
    /// Only publication metadata moved.
    Metadata,
    /// Roster movement, or a Federal Register notice with no CFR effect.
    Routine,
    /// A non-substantive amendment as eCFR itself classifies it, or a proposed
    /// rule that has not taken effect.
    Minor,
    /// A substantive amendment to Title 34, or a section removed.
    Major,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Metadata => "metadata",
            Self::Routine => "routine",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }

    pub fn headline(self) -> &'static str {
        match self {
            Self::None => "no change",
            Self::Metadata => "metadata only",
            Self::Routine => "routine movement",
            Self::Minor => "non-substantive amendment",
            Self::Major => "TITLE 34 CHANGED",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "metadata" => Ok(Self::Metadata),
            "routine" => Ok(Self::Routine),
            "minor" => Ok(Self::Minor),
            "major" => Ok(Self::Major),
            other => Err(format!("unknown severity `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_lets_max_pick_the_loudest_source() {
        let observed = [Severity::Routine, Severity::Major, Severity::Metadata];
        assert_eq!(observed.into_iter().max(), Some(Severity::Major));
    }

    #[test]
    fn a_substantive_amendment_outranks_a_roster_refresh() {
        assert!(Severity::Major > Severity::Routine);
        assert!(Severity::Minor > Severity::Metadata);
    }

    #[test]
    fn parses_from_cli_text() {
        assert_eq!("Major".parse::<Severity>().unwrap(), Severity::Major);
        assert!("loud".parse::<Severity>().is_err());
    }
}
