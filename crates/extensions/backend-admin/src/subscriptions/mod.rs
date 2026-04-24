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

//! Admin HTTP surface for the kernel `SubscriptionRegistry`.
//!
//! Tag-based notification subscriptions already exist inside the kernel,
//! but have no out-of-band management channel — without a REST surface
//! the feed dispatch loop (`crates/app/src/lib.rs`) finds nothing to match
//! and no `ProactiveTurn` ever fires. This module wraps the registry with
//! a thin HTTP facade so operators can create / list / update / delete
//! subscriptions at runtime.

mod router;

pub use router::{SubscriptionRouterState, subscription_routes};
