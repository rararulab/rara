// Copyright 2025 Crrow
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

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

            /// Return the inner [`Uuid`].
            #[must_use]
            pub fn into_inner(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

define_id!(
    /// Unique identifier for a job source (e.g. LinkedIn, Indeed).
    JobSourceId
);

define_id!(
    /// Unique identifier for a resume version.
    ResumeId
);

define_id!(
    /// Unique identifier for a job application.
    ApplicationId
);

define_id!(
    /// Unique identifier for an interview.
    InterviewId
);

define_id!(
    /// Unique identifier for a scheduler task.
    SchedulerTaskId
);
