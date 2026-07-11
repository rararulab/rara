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

//! Shared timeout coercion for sandbox exec tools.
//!
//! Both `bash` and `run_code` accept an optional per-call `timeout` param
//! that LLMs express in several shapes. This module owns the single
//! `deserialize_timeout` visitor so the two tools cannot drift.

use std::{fmt, time::Duration};

use serde::{
    Deserializer,
    de::{self, Visitor},
};

/// Accept `30` (integer), `"30"` (stringified integer), `"30s"` / `"2m"`
/// (humantime duration), or `{"secs": N, "nanos": N}` (Duration struct
/// layout that some LLMs emit) for a `timeout` field.
pub(crate) fn deserialize_timeout<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    struct TimeoutVisitor;

    impl<'de> Visitor<'de> for TimeoutVisitor {
        type Value = Option<Duration>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an integer, stringified integer, or humantime duration")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }

        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(DurationVisitor).map(Some)
        }
    }

    struct DurationVisitor;

    impl<'de> Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str(
                "an integer (seconds), stringified integer, humantime duration, or {\"secs\": N, \
                 \"nanos\": N} map",
            )
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Duration, E> {
            Ok(Duration::from_secs(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Duration, E> {
            let secs = u64::try_from(v).map_err(|_| E::custom(format!("negative timeout: {v}")))?;
            Ok(Duration::from_secs(secs))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Duration, E> {
            let s = v.trim();
            // Try bare integer first ("30" → 30 seconds).
            if let Ok(secs) = s.parse::<u64>() {
                return Ok(Duration::from_secs(secs));
            }
            // Fall back to humantime ("30s", "2m").
            humantime::parse_duration(s).map_err(|_| E::custom(format!("invalid timeout: {v:?}")))
        }

        /// Accept `{"secs": 30, "nanos": 0}` — the Duration struct layout
        /// that some LLMs (e.g. GPT-5.4) emit when they see the JSON schema.
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Duration, A::Error> {
            let mut secs: Option<u64> = None;
            let mut nanos: Option<u32> = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "secs" => secs = Some(map.next_value()?),
                    "nanos" => nanos = Some(map.next_value()?),
                    _ => {
                        let _ = map.next_value::<de::IgnoredAny>()?;
                    }
                }
            }

            let secs = secs.ok_or_else(|| de::Error::missing_field("secs"))?;
            Ok(Duration::new(secs, nanos.unwrap_or(0)))
        }
    }

    deserializer.deserialize_option(TimeoutVisitor)
}
