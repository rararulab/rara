//! Strongly-typed domain identifiers.
//!
//! Each aggregate root gets its own newtype wrapper around [`uuid::Uuid`] so
//! that IDs from different domains cannot be accidentally mixed up.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
            utoipa::ToSchema,
        )]
        #[display("{_0}")]
        #[schema(value_type = String, format = "uuid")]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
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
