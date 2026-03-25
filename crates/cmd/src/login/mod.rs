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

//! `rara login` subcommand — provider authentication.

use clap::{Args, Subcommand};
use snafu::{FromString, Whatever};

/// Provider authentication commands.
#[derive(Debug, Clone, Args)]
#[command(about = "Authenticate with LLM providers")]
pub struct LoginCmd {
    #[command(subcommand)]
    sub: LoginSub,
}

#[derive(Debug, Clone, Subcommand)]
enum LoginSub {
    /// Authenticate with OpenAI via Codex OAuth (PKCE flow).
    Codex,
}

impl LoginCmd {
    /// Execute the selected login subcommand.
    pub async fn run(self) -> Result<(), Whatever> {
        match self.sub {
            LoginSub::Codex => run_codex_login().await,
        }
    }
}

async fn run_codex_login() -> Result<(), Whatever> {
    use rara_codex_oauth::*;

    println!("Starting Codex OAuth login...\n");

    // Generate PKCE verifier + challenge pair for the authorization flow.
    let state = generate_nonce();
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    // Persist pending state so the callback server can validate later.
    save_pending_oauth(&PendingCodexOAuth {
        state: state.clone(),
        code_verifier,
    })
    .map_err(|e| Whatever::without_source(e.to_string()))?;

    // Spin up the ephemeral callback server before sending the user to the
    // browser, so it is ready to receive the redirect.
    start_callback_server()
        .await
        .map_err(|e| Whatever::without_source(e.to_string()))?;

    // Build the authorization URL with PKCE challenge.
    let auth_url = build_auth_url(&state, &code_challenge)
        .map_err(|e| Whatever::without_source(e.to_string()))?;

    println!("Open this URL in your browser to authorize:\n");
    println!("  {auth_url}\n");

    println!("Waiting for authorization...");

    // Remember the existing token timestamp so we can detect a fresh exchange.
    let previous_expiry = load_tokens()
        .await
        .ok()
        .flatten()
        .and_then(|t| t.expires_at_unix);

    // Poll for tokens — the callback server saves them on successful exchange.
    let tokens = loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        match load_tokens().await {
            Ok(Some(tokens)) if tokens.expires_at_unix != previous_expiry => break tokens,
            Ok(Some(_) | None) => continue,
            Err(e) => {
                return Err(Whatever::without_source(format!(
                    "failed to check tokens: {e}"
                )));
            }
        }
    };

    println!("\nLogin successful!");
    if let Some(expires_at) = tokens.expires_at_unix {
        let duration = expires_at.saturating_sub(now_unix());
        println!("Token expires in {} minutes.", duration / 60);
    }

    println!("\nTo use Codex as your LLM provider, add to your config.yaml:\n");
    println!("  llm:");
    println!("    default_provider: codex");
    println!("    providers:");
    println!("      codex:");
    println!("        default_model: gpt-4.1");
    println!("\nThen restart rara.");

    Ok(())
}
