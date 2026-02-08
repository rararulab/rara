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

use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(
        std::env::var("OUT_DIR")
            .expect("cargo built-in env value 'OUT_DIR' must be set during compilation"),
    );

    // const EQ_ATTR: &str = "#[derive(serde::Serialize, serde::Deserialize,  Eq)]";

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("rsketch_grpc_desc.bin"))
        // .type_attribute("rsketch.v1.hello.Message", EQ_ATTR)
        .compile_protos(&[
            "proto/hello/v1/hello.proto",
        ], &["proto"])
        .expect("compile proto");
}
