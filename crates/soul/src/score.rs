// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Bounded 1-10 score type for soul style parameters.

use std::fmt;

use serde::{Deserialize, Serialize};
use snafu::Snafu;

/// Error returned when a score value is outside the valid 1-10 range.
#[derive(Debug, Snafu)]
#[snafu(display("style score {value} out of range 1-10"))]
pub struct StyleScoreError {
    pub(crate) value: u8,
}

/// A score value bounded to the 1-10 range (inclusive).
///
/// Used for style drift parameters (formality, verbosity, humor) and
/// boundary formality limits. Deserialization via `serde(try_from)` ensures
/// invalid values are rejected at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct StyleScore(u8);

impl StyleScore {
    /// Create a new `StyleScore`, returning an error if `value` is outside
    /// 1-10.
    pub fn new(value: u8) -> Result<Self, StyleScoreError> {
        if value == 0 || value > 10 {
            Err(StyleScoreError { value })
        } else {
            Ok(Self(value))
        }
    }

    /// Return the inner `u8` value.
    pub fn get(self) -> u8 { self.0 }
}

impl TryFrom<u8> for StyleScore {
    type Error = StyleScoreError;

    fn try_from(value: u8) -> Result<Self, Self::Error> { Self::new(value) }
}

impl From<StyleScore> for u8 {
    fn from(s: StyleScore) -> u8 { s.0 }
}

impl Default for StyleScore {
    fn default() -> Self {
        // Midpoint of 1-10.
        Self(5)
    }
}

impl fmt::Display for StyleScore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_range() {
        for v in 1..=10 {
            assert!(StyleScore::new(v).is_ok());
        }
    }

    #[test]
    fn zero_rejected() {
        assert!(StyleScore::new(0).is_err());
    }

    #[test]
    fn above_ten_rejected() {
        assert!(StyleScore::new(11).is_err());
        assert!(StyleScore::new(255).is_err());
    }

    #[test]
    fn default_is_midpoint() {
        assert_eq!(StyleScore::default().get(), 5);
    }

    #[test]
    fn ordering() {
        let low = StyleScore::new(2).unwrap();
        let high = StyleScore::new(9).unwrap();
        assert!(low < high);
    }

    #[test]
    fn serde_roundtrip() {
        let score = StyleScore::new(7).unwrap();
        let yaml = serde_yaml::to_string(&score).unwrap();
        let parsed: StyleScore = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, score);
    }

    #[test]
    fn serde_rejects_zero() {
        let result: Result<StyleScore, _> = serde_yaml::from_str("0");
        assert!(result.is_err());
    }

    #[test]
    fn serde_rejects_eleven() {
        let result: Result<StyleScore, _> = serde_yaml::from_str("11");
        assert!(result.is_err());
    }
}
