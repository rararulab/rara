use std::sync::Arc;

use async_trait::async_trait;
use job_base::shared_string::SharedString;
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
                let api_key = job_base::env::required_var(OPENROUTER_API_KEY_ENV)
                    .ok()
                    .context(OpenRouterNotConfiguredSnafu)?;

                Self::build_client(api_key)
            })
            .await?;

        Ok(client_ref.clone())
    }
}
