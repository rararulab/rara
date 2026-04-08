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

//! Strongly-typed domain identifiers.
//!
//! Each aggregate root gets its own newtype wrapper around a
//! [`std::num::NonZeroU128`] storing a UUID. The `NonZero` representation
//! enables niche optimisation so that `Option<Id>` is the same size as `Id`
//! (16 bytes), without changing the on-the-wire / on-disk format — `Display`
//! and `serde` continue to use the canonical UUID string representation.

#[macro_export]
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(std::num::NonZeroU128);

        impl $name {
            /// Generate a new random identifier (UUID v4).
            #[must_use]
            pub fn new() -> Self {
                let uuid = uuid::Uuid::new_v4();
                // SAFETY: a v4 UUID has its variant + version bits set, so the
                // u128 representation is always non-zero.
                Self(
                    std::num::NonZeroU128::new(uuid.as_u128())
                        .expect("v4 UUID is never zero"),
                )
            }

            /// Generate a deterministic identifier from a name string.
            ///
            /// Uses UUID v5 (SHA-1) with the OID namespace so the same name
            /// always produces the same identifier.
            #[must_use]
            pub fn deterministic(name: &str) -> Self {
                let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes());
                // SAFETY: a v5 UUID has its variant + version bits set, so the
                // u128 representation is always non-zero.
                Self(
                    std::num::NonZeroU128::new(uuid.as_u128())
                        .expect("v5 UUID is never zero"),
                )
            }

            /// Return the inner [`uuid::Uuid`].
            #[must_use]
            pub fn as_uuid(&self) -> uuid::Uuid {
                uuid::Uuid::from_u128(self.0.get())
            }

            /// Consume the identifier and return the underlying [`uuid::Uuid`].
            #[must_use]
            pub fn into_inner(self) -> uuid::Uuid {
                self.as_uuid()
            }

            /// Construct an identifier from an existing [`uuid::Uuid`].
            ///
            /// Returns `None` for the nil (all-zero) UUID.
            #[must_use]
            pub fn from_uuid(uuid: uuid::Uuid) -> Option<Self> {
                std::num::NonZeroU128::new(uuid.as_u128()).map(Self)
            }

            /// Try to parse an identifier from a string, returning an error on
            /// invalid UUID or on the nil UUID.
            pub fn try_from_raw(raw: &str) -> Result<Self, uuid::Error> {
                let uuid = uuid::Uuid::parse_str(raw)?;
                // The nil UUID parses successfully but is invalid for our
                // domain identifiers. Re-parse the string "" to fabricate the
                // canonical "invalid character" error.
                Self::from_uuid(uuid).ok_or_else(|| {
                    uuid::Uuid::parse_str("").expect_err("empty string is never a valid UUID")
                })
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.as_uuid().fmt(f)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.as_uuid().serialize(s)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let uuid = uuid::Uuid::deserialize(d)?;
                Self::from_uuid(uuid)
                    .ok_or_else(|| <D::Error as serde::de::Error>::custom("nil UUID"))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    crate::define_id!(TestId);

    #[test]
    fn option_is_niche_optimized() {
        assert_eq!(size_of::<TestId>(), 16);
        assert_eq!(size_of::<Option<TestId>>(), 16, "niche optimization failed");
    }

    #[test]
    fn round_trip_display_and_parse() {
        let id = TestId::new();
        let s = id.to_string();
        let parsed = TestId::try_from_raw(&s).expect("parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn deterministic_is_stable() {
        let a = TestId::deterministic("hello");
        let b = TestId::deterministic("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn from_nil_uuid_is_none() {
        assert!(TestId::from_uuid(uuid::Uuid::nil()).is_none());
    }

    #[test]
    fn serde_round_trip_is_uuid_string() {
        let id = TestId::new();
        let json = serde_json::to_string(&id).unwrap();
        // The serialized form must be a quoted UUID string.
        assert_eq!(json, format!("\"{}\"", id.as_uuid()));
        let back: TestId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
