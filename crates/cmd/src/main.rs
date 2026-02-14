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

use snafu::{ResultExt, Whatever};

use rara_app::AppConfig;

#[tokio::main]
async fn main() -> Result<(), Whatever> {
    let _guards = common_telemetry::logging::init_tracing_subscriber("rara");
    let config = AppConfig::new().whatever_context("Failed to load config")?;
    config.run().await
}
