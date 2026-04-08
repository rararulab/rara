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

//! Derive macro for type-safe agent tool definitions.
//!
//! Generates `AgentTool` implementations from a single annotated struct,
//! using `schemars::JsonSchema` for schema generation and `serde::Deserialize`
//! for parameter parsing.
//!
//! Runtime helpers (`clean_schema`, `ToolExecute`, `EmptyParams`) live in
//! `rara_kernel::tool` — this crate is proc-macro only.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr, LitBool, LitStr, Token, parse_macro_input};

/// Derive macro that generates an `AgentTool` implementation for a tool struct.
///
/// # Attributes
///
/// - `name` (required): The tool name string.
/// - `description` (required): The tool description string.
/// - `params_schema` (optional): Expression returning `serde_json::Value`.
///   Defaults to generating schema from `<Self as ToolExecute>::Params`.
/// - `execute_fn` (optional): Path to a custom execute function with signature
///   `async fn(&self, Value, &ToolContext) -> Result<ToolOutput>`. Defaults to
///   deserializing params and calling `ToolExecute::run`.
/// - `validate_fn` (optional): Path to a custom validate function with
///   signature `async fn(&Value) -> anyhow::Result<()>`. Use this with
///   `execute_fn` mode (no `ToolExecute` impl). When `ToolExecute` is present,
///   `ToolExecute::validate` is auto-bridged and this attribute is not needed.
/// - `manual_impl` (optional): If `true`, only generates `TOOL_NAME` and
///   `TOOL_DESCRIPTION` constants; user writes `impl AgentTool` manually.
#[proc_macro_derive(ToolDef, attributes(tool))]
pub fn derive_tool_def(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_tool_def(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed `#[tool(...)]` attributes.
#[allow(clippy::struct_excessive_bools)]
struct ToolAttrs {
    name:             LitStr,
    description:      LitStr,
    params_schema:    Option<Expr>,
    execute_fn:       Option<Expr>,
    validate_fn:      Option<Expr>,
    manual_impl:      bool,
    tier:             Option<LitStr>,
    timeout_secs:     Option<syn::LitInt>,
    read_only:        bool,
    destructive:      bool,
    concurrency_safe: bool,
    user_interaction: bool,
}

fn parse_tool_attrs(input: &DeriveInput) -> syn::Result<ToolAttrs> {
    let mut name: Option<LitStr> = None;
    let mut description: Option<LitStr> = None;
    let mut params_schema: Option<Expr> = None;
    let mut execute_fn: Option<Expr> = None;
    let mut validate_fn: Option<Expr> = None;
    let mut manual_impl = false;
    let mut tier: Option<LitStr> = None;
    let mut timeout_secs: Option<syn::LitInt> = None;
    let mut read_only = false;
    let mut destructive = false;
    let mut concurrency_safe = false;
    let mut user_interaction = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                meta.input.parse::<Token![=]>()?;
                name = Some(meta.input.parse::<LitStr>()?);
            } else if meta.path.is_ident("description") {
                meta.input.parse::<Token![=]>()?;
                description = Some(meta.input.parse::<LitStr>()?);
            } else if meta.path.is_ident("params_schema") {
                meta.input.parse::<Token![=]>()?;
                let lit: LitStr = meta.input.parse()?;
                params_schema = Some(lit.parse::<Expr>()?);
            } else if meta.path.is_ident("execute_fn") {
                meta.input.parse::<Token![=]>()?;
                let lit: LitStr = meta.input.parse()?;
                execute_fn = Some(lit.parse::<Expr>()?);
            } else if meta.path.is_ident("validate_fn") {
                meta.input.parse::<Token![=]>()?;
                let lit: LitStr = meta.input.parse()?;
                validate_fn = Some(lit.parse::<Expr>()?);
            } else if meta.path.is_ident("manual_impl") {
                meta.input.parse::<Token![=]>()?;
                let lit: LitBool = meta.input.parse()?;
                manual_impl = lit.value();
            } else if meta.path.is_ident("tier") {
                meta.input.parse::<Token![=]>()?;
                tier = Some(meta.input.parse::<LitStr>()?);
            } else if meta.path.is_ident("timeout_secs") {
                meta.input.parse::<Token![=]>()?;
                timeout_secs = Some(meta.input.parse::<syn::LitInt>()?);
            } else if meta.path.is_ident("read_only") {
                read_only = true;
            } else if meta.path.is_ident("destructive") {
                destructive = true;
            } else if meta.path.is_ident("concurrency_safe") {
                concurrency_safe = true;
            } else if meta.path.is_ident("user_interaction") {
                user_interaction = true;
            } else {
                return Err(meta.error(format!(
                    "unknown tool attribute: `{}`",
                    meta.path
                        .get_ident()
                        .map_or_else(|| "?".to_string(), ToString::to_string)
                )));
            }
            Ok(())
        })?;
    }

    let name = name
        .ok_or_else(|| syn::Error::new_spanned(input, "missing required `name` in #[tool(...)]"))?;
    let description = description.ok_or_else(|| {
        syn::Error::new_spanned(input, "missing required `description` in #[tool(...)]")
    })?;

    Ok(ToolAttrs {
        name,
        description,
        params_schema,
        execute_fn,
        validate_fn,
        manual_impl,
        tier,
        timeout_secs,
        read_only,
        destructive,
        concurrency_safe,
        user_interaction,
    })
}

fn expand_tool_def(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let attrs = parse_tool_attrs(input)?;
    let struct_name = &input.ident;
    let tool_name = &attrs.name;
    let tool_desc = &attrs.description;

    // Always generate constants.
    let constants = quote! {
        impl #struct_name {
            /// The tool name constant.
            pub const TOOL_NAME: &'static str = #tool_name;
            /// The tool description constant.
            pub const TOOL_DESCRIPTION: &'static str = #tool_desc;
        }
    };

    if attrs.manual_impl {
        return Ok(constants);
    }

    // Build parameters_schema body.
    let schema_body = if let Some(expr) = &attrs.params_schema {
        quote! { #expr }
    } else {
        quote! {
            crate::tool::clean_schema(
                schemars::schema_for!(<Self as crate::tool::ToolExecute>::Params)
            )
        }
    };

    // Build execute body.
    let execute_body = if let Some(expr) = &attrs.execute_fn {
        quote! {
            #expr(params, context).await
        }
    } else {
        quote! {
            let typed: <Self as crate::tool::ToolExecute>::Params =
                serde_json::from_value(params)
                    .map_err(|e| anyhow::anyhow!("invalid params for '{}': {e}", self.name()))?;
            // Note: for `type Output = serde_json::Value` this clones the Value
            // (serde_json::to_value on a Value is a clone). Acceptable tradeoff
            // for uniform codegen; typed Output structs serialize exactly once.
            let output = crate::tool::ToolExecute::run(self, typed, context).await?;
            crate::tool::ToolOutput::from_serialize(&output)
        }
    };

    // Build validate body. Three modes:
    //   1. `validate_fn = "..."` set       → call user fn directly
    //   2. otherwise, if execute uses ToolExecute → bridge to ToolExecute::validate
    //      (deserialise once for the typed call; we accept the parse cost because
    //      validate is the right place to surface schema errors)
    //   3. otherwise (execute_fn mode without validate_fn) → omit, trait default
    //      applies
    let validate_impl = if let Some(expr) = &attrs.validate_fn {
        quote! {
            async fn validate(
                &self,
                params: &serde_json::Value,
            ) -> anyhow::Result<()> {
                #expr(params).await
            }
        }
    } else if attrs.execute_fn.is_none() {
        quote! {
            async fn validate(
                &self,
                params: &serde_json::Value,
            ) -> anyhow::Result<()> {
                let typed: <Self as crate::tool::ToolExecute>::Params =
                    serde_json::from_value(params.clone())
                        .map_err(|e| anyhow::anyhow!("invalid params for '{}': {e}", self.name()))?;
                crate::tool::ToolExecute::validate(self, &typed).await
            }
        }
    } else {
        quote! {}
    };

    let timeout_impl = match &attrs.timeout_secs {
        Some(lit) => quote! {
            fn execution_timeout(&self) -> Option<std::time::Duration> {
                Some(std::time::Duration::from_secs(#lit))
            }
        },
        None => quote! {},
    };

    let tier_impl = match &attrs.tier {
        Some(lit) if lit.value() == "deferred" => quote! {
            fn tier(&self) -> crate::tool::ToolTier { crate::tool::ToolTier::Deferred }
        },
        Some(lit) if lit.value() == "core" => quote! {},
        Some(lit) => {
            return Err(syn::Error::new_spanned(
                lit,
                format!(
                    "unknown tier value `{}`: expected \"core\" or \"deferred\"",
                    lit.value()
                ),
            ));
        }
        None => quote! {},
    };

    // Safety axes — only override when the flag is set (otherwise trait
    // default `false` applies, which is the fail-closed behaviour we want).
    let read_only_impl = if attrs.read_only {
        quote! {
            fn is_read_only(&self, _args: &serde_json::Value) -> bool { true }
        }
    } else {
        quote! {}
    };
    let destructive_impl = if attrs.destructive {
        quote! {
            fn is_destructive(&self, _args: &serde_json::Value) -> bool { true }
        }
    } else {
        quote! {}
    };
    let concurrency_safe_impl = if attrs.concurrency_safe {
        quote! {
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool { true }
        }
    } else {
        quote! {}
    };
    let user_interaction_impl = if attrs.user_interaction {
        quote! {
            fn requires_user_interaction(&self) -> bool { true }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #constants

        #[async_trait::async_trait]
        impl crate::tool::AgentTool for #struct_name {
            fn name(&self) -> &str { Self::TOOL_NAME }

            fn description(&self) -> &str { Self::TOOL_DESCRIPTION }

            fn parameters_schema(&self) -> serde_json::Value {
                #schema_body
            }

            #validate_impl

            #read_only_impl
            #destructive_impl
            #concurrency_safe_impl
            #user_interaction_impl

            async fn execute(
                &self,
                params: serde_json::Value,
                context: &crate::tool::ToolContext,
            ) -> anyhow::Result<crate::tool::ToolOutput> {
                #execute_body
            }

            #tier_impl

            #timeout_impl
        }
    };

    Ok(expanded)
}
