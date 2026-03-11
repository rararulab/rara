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
//! Each aggregate root gets its own newtype wrapper around [`uuid::Uuid`] so
//! that IDs from different domains cannot be accidentally mixed up.

#[macro_export]
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            Serialize,
            Deserialize,
            derive_more::Display,
            derive_more::From,
        )]
        #[display("{_0}")]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Generate a deterministic identifier from a name string.
            ///
            /// Uses UUID v5 (SHA-1) with the OID namespace so the same name
            /// always produces the same identifier.
            #[must_use]
            pub fn deterministic(name: &str) -> Self {
                Self(Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()))
            }

            /// Return the inner [`Uuid`].
            #[must_use]
            pub fn into_inner(self) -> Uuid {
                self.0
            }

            /// Try to parse an identifier from a string, returning an error on
            /// invalid UUID.
            pub fn try_from_raw(raw: &str) -> Result<Self, uuid::Error> {
                uuid::Uuid::parse_str(raw).map(Self)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}
