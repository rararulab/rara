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
//! Generates [`AgentTool`] implementations from a single annotated struct,
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
struct ToolAttrs {
    name:          LitStr,
    description:   LitStr,
    params_schema: Option<Expr>,
    execute_fn:    Option<Expr>,
    manual_impl:   bool,
}

fn parse_tool_attrs(input: &DeriveInput) -> syn::Result<ToolAttrs> {
    let mut name: Option<LitStr> = None;
    let mut description: Option<LitStr> = None;
    let mut params_schema: Option<Expr> = None;
    let mut execute_fn: Option<Expr> = None;
    let mut manual_impl = false;

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
            } else if meta.path.is_ident("manual_impl") {
                meta.input.parse::<Token![=]>()?;
                let lit: LitBool = meta.input.parse()?;
                manual_impl = lit.value();
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
        manual_impl,
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
            crate::tool::ToolExecute::run(self, typed, context).await
        }
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

            async fn execute(
                &self,
                params: serde_json::Value,
                context: &crate::tool::ToolContext,
            ) -> anyhow::Result<crate::tool::ToolOutput> {
                #execute_body
            }
        }
    };

    Ok(expanded)
}
