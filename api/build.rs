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

use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(
        std::env::var("OUT_DIR")
            .expect("cargo built-in env value 'OUT_DIR' must be set during compilation"),
    );

    let mut includes: Vec<PathBuf> = vec!["proto".into()];

    // Add system protobuf include path for well-known types (e.g.
    // google/protobuf/timestamp.proto)
    if let Some(path) = std::env::var_os("PROTOC_INCLUDE") {
        includes.push(path.into());
    } else {
        for candidate in ["/usr/include", "/usr/local/include"] {
            let p = PathBuf::from(candidate);
            if p.join("google/protobuf/timestamp.proto").exists() {
                includes.push(p);
                break;
            }
        }
    }

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("rara_grpc_desc.bin"))
        .compile_protos(
            &[
                "proto/hello/v1/hello.proto",
                "proto/telegrambot/v1/command.proto",
                "proto/execution/v1/worker.proto",
            ],
            &includes
                .iter()
                .map(|p| p.to_str().unwrap())
                .collect::<Vec<_>>(),
        )
        .expect("compile proto");
}
