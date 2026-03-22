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

//! Tape memory tools — expose the raw tape subsystem to the LLM agent.
//!
//! Each tool is a focused, single-purpose operation on the session tape.
//! Registered per-session via `SyscallTool` in the kernel syscall handler.

mod anchor;
mod anchors;
mod between;
mod checkout;
mod checkout_root;
mod entries;
mod info;
mod search;

pub(crate) use anchor::TapeAnchorTool;
pub(crate) use anchors::TapeAnchorsTool;
pub(crate) use between::TapeBetweenTool;
pub(crate) use checkout::TapeCheckoutTool;
pub(crate) use checkout_root::TapeCheckoutRootTool;
pub(crate) use entries::TapeEntriesTool;
pub(crate) use info::TapeInfoTool;
pub(crate) use search::TapeSearchTool;
