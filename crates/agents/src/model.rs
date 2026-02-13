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

use std::sync::Arc;

use async_trait::async_trait;
use base::shared_string::SharedString;
use openrouter_rs::client::OpenRouterClient;
use snafu::{OptionExt, ResultExt};
use tokio::sync::OnceCell;

use crate::err::prelude::*;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

pub type OpenRouterLoaderRef = Arc<dyn OpenRouterLoader>;

pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_KEY";

#[async_trait]
pub trait OpenRouterLoader {
    async fn acquire_client(&self) -> Result<OpenRouterClient>;

    fn build_client<S: AsRef<str>>(api_key: S) -> Result<OpenRouterClient>
    where
        Self: Sized,
    {
        OpenRouterClient::builder()
            .api_key(api_key.as_ref())
            .build()
            .context(OpenRouterSnafu)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvOpenRouterLoader {
    client: Arc<OnceCell<OpenRouterClient>>,
}

#[async_trait]
impl OpenRouterLoader for EnvOpenRouterLoader {
    async fn acquire_client(&self) -> Result<OpenRouterClient> {
        let client_ref = self
            .client
            .get_or_try_init(|| async {
                let api_key = base::env::required_var(OPENROUTER_API_KEY_ENV)
                    .ok()
                    .context(OpenRouterNotConfiguredSnafu)?;

                Self::build_client(api_key)
            })
            .await?;

        Ok(client_ref.clone())
    }
}
