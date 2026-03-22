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

//! `rara wechat` subcommand — WeChat iLink Bot management.

use clap::{Args, Subcommand};
use snafu::{FromString, Whatever};

/// WeChat iLink Bot management commands.
#[derive(Debug, Clone, Args)]
#[command(about = "WeChat iLink Bot management")]
pub struct WechatCmd {
    #[command(subcommand)]
    sub: WechatSub,
}

#[derive(Debug, Clone, Subcommand)]
enum WechatSub {
    /// Interactive QR-code login — scan with WeChat to authenticate.
    Login(LoginArgs),
}

#[derive(Debug, Clone, Args)]
struct LoginArgs {
    /// Override the iLink API base URL.
    #[arg(long)]
    base_url: Option<String>,
}

impl WechatCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        match self.sub {
            WechatSub::Login(args) => args.run().await,
        }
    }
}

impl LoginArgs {
    async fn run(self) -> Result<(), Whatever> {
        let account_id = rara_channels::wechat::login::login(self.base_url.as_deref())
            .await
            .map_err(|e| Whatever::without_source(format!("{e}")))?;

        println!("\nAdd this to ~/.config/rara/config.yaml:\n");
        println!("wechat:");
        println!("  account_id: \"{account_id}\"");
        println!("\nThen restart rara to activate the WeChat channel.");

        Ok(())
    }
}
