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

pub use serde;

pub mod pb {
    #![allow(
        clippy::missing_errors_doc,
        clippy::default_trait_access,
        clippy::similar_names,
        clippy::doc_markdown,
        clippy::missing_const_for_fn
    )]

    pub const GRPC_DESC: &[u8] = tonic::include_file_descriptor_set!("rsketch_grpc_desc");

    pub mod hello {
        pub mod v1 {
            tonic::include_proto!("rsketch.hello.v1");
        }
    }
}
