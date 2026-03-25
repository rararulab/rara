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
use snafu::{ResultExt as _, Whatever};

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

    // Build the authorization URL with PKCE challenge.
    let auth_url =
        build_auth_url(&state, &code_challenge).whatever_context("failed to build auth URL")?;

    // Try to open the browser automatically; fall back to printing the URL.
    if open::that(&auth_url).is_ok() {
        println!("Opened your browser for authorization.\n");
        println!("If it didn't open, visit this URL manually:\n");
    } else {
        println!("Open this URL in your browser to authorize:\n");
    }
    println!("  {auth_url}\n");

    // The OAuth provider redirects the browser to localhost:1455/auth/callback,
    // which fails when the browser runs on a different machine or behind a proxy
    // that intercepts localhost traffic. Instead of relying on a local HTTP
    // server, ask the user to copy the full redirect URL from their browser's
    // address bar — we parse the code out and complete the exchange locally.
    println!("After authorizing, your browser will redirect to:");
    println!("  http://localhost:1455/auth/callback?code=...&state=...\n");
    println!("The page will show a connection error — that is expected.");
    println!("Copy the full URL from your browser's address bar and paste it below.\n");

    let callback_url = read_stdin_line("Callback URL: ")
        .await
        .whatever_context("failed to read callback URL")?;

    if callback_url.is_empty() {
        snafu::whatever!("no callback URL provided");
    }

    let (code, returned_state) =
        parse_callback_url(&callback_url).whatever_context("failed to parse callback URL")?;

    validate_state(&state, &returned_state)
        .whatever_context("OAuth state mismatch — please retry the login")?;

    println!("\nExchanging authorization code for tokens...");

    let tokens = exchange_authorization_code(&code, &code_verifier)
        .await
        .whatever_context("failed to exchange authorization code")?;

    save_tokens(&tokens)
        .await
        .whatever_context("failed to save tokens")?;

    println!("\nLogin successful!");
    if let Some(expires_at) = tokens.expires_at_unix {
        let duration = expires_at.saturating_sub(now_unix());
        println!("Token expires in {} minutes.", duration / 60);
    }

    println!("\nTo use Codex as your LLM provider, add to your config.yaml:");
    println!("  (typically at ~/.config/rara/config.yaml)\n");
    println!("  llm:");
    println!("    default_provider: codex");
    println!("    providers:");
    println!("      codex:");
    println!("        default_model: gpt-4.1");
    println!("\nThen restart rara.");

    Ok(())
}

/// Read one line from stdin without blocking the tokio executor.
async fn read_stdin_line(prompt: &str) -> Result<String, Whatever> {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout()
        .flush()
        .whatever_context("failed to flush stdout")?;
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .whatever_context("failed to read from stdin")?;
        Ok(line.trim().to_owned())
    })
    .await
    .whatever_context("stdin reader panicked")?
}
