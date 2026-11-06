//! The Jev score a rule's judged things must reach to keep the rule.

use std::fmt;
use std::str::FromStr;

use crate::stats::FESTIVE;

/// A score from 0 to 1. Scores at or above it keep the rule; below it break it. Only
/// [`Threshold::new`] makes one, so a threshold outside 0..=1 (or NaN) can't exist.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Threshold(f32);

impl Threshold {
    /// For rules without their own threshold.
    pub const DEFAULT: Self = Self(FESTIVE);

    pub fn new(value: f32) -> Option<Self> {
        (0.0..=1.0).contains(&value).then_some(Self(value))
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// Does `score` keep a rule with this threshold? The one place pass/fail is decided.
    pub fn kept_by(self, score: f32) -> bool {
        score >= self.0
    }
}

impl Default for Threshold {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for Threshold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

/// An admin's threshold setting: a number from 0 to 1, or "default" (`None`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThresholdSetting(pub Option<Threshold>);

impl FromStr for ThresholdSetting {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("default") {
            return Ok(Self(None));
        }
        s.parse()
            .ok()
            .and_then(Threshold::new)
            .map(|t| Self(Some(t)))
            .ok_or_else(|| {
                format!("`{s}` isn't a threshold: use a number from 0 to 1, or `default`")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: f32) -> Threshold {
        Threshold::new(v).unwrap()
    }

    #[test]
    fn only_scores_from_zero_to_one_are_thresholds() {
        assert_eq!(Threshold::new(0.0).map(Threshold::get), Some(0.0));
        assert_eq!(Threshold::new(0.3).map(Threshold::get), Some(0.3));
        assert_eq!(Threshold::new(1.0).map(Threshold::get), Some(1.0));
        for nonsense in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            assert_eq!(Threshold::new(nonsense), None, "{nonsense}");
        }
    }

    #[test]
    fn a_score_at_the_threshold_keeps_the_rule_and_just_below_breaks_it() {
        assert!(t(0.3).kept_by(0.3));
        assert!(t(0.3).kept_by(0.31));
        assert!(!t(0.3).kept_by(0.29));
        assert!(Threshold::default().kept_by(FESTIVE));
        assert!(!Threshold::default().kept_by(FESTIVE - 0.01));
    }

    #[test]
    fn admins_set_a_number_from_zero_to_one_or_default() {
        let parse = |s: &str| s.parse::<ThresholdSetting>().map(|s| s.0);
        assert_eq!(parse("0.35"), Ok(Some(t(0.35))));
        assert_eq!(parse(" 1 "), Ok(Some(t(1.0))));
        assert_eq!(parse("default"), Ok(None));
        assert_eq!(parse("Default"), Ok(None));
        for bad in ["1.5", "-0.2", "NaN", "inf", "lots", ""] {
            assert!(parse(bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn thresholds_show_two_decimals() {
        assert_eq!(t(0.5).to_string(), "0.50");
    }
}
