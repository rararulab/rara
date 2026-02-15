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

//! Typst compilation engine.
//!
//! Wraps `typst-as-lib` to compile a set of in-memory `.typ` files into a
//! PDF document. The compiler is stateless: each call receives a file map
//! and returns PDF bytes (or a compilation error).

use std::{collections::HashMap, path::Path};

use typst_as_lib::TypstEngine;

use crate::error::TypstError;

/// Compile Typst source files into PDF bytes.
///
/// # Arguments
///
/// * `files` - Map of relative path -> file content (e.g. `"main.typ" ->
///   "..."`).
/// * `main_file` - The entry-point file path (must exist in `files`).
///
/// # Returns
///
/// A tuple of `(pdf_bytes, page_count)` on success.
pub fn compile(
    files: &HashMap<String, String>,
    main_file: &str,
) -> Result<(Vec<u8>, usize), TypstError> {
    if !files.contains_key(main_file) {
        return Err(TypstError::InvalidRequest {
            message: format!("main file not found in project files: {main_file}"),
        });
    }

    // Build file list as (path, content) tuples for the static source resolver.
    let source_files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();

    let engine = TypstEngine::builder()
        .with_static_source_file_resolver(source_files)
        .build();

    compile_engine(&engine, main_file)
}

/// Compile a Typst project from a local directory.
///
/// Uses the filesystem resolver so that `#import` and package references
/// (e.g. `@preview/...`) are resolved naturally by Typst.
///
/// # Arguments
///
/// * `root` - The project root directory on disk.
/// * `main_file` - The entry-point file path relative to root.
pub fn compile_from_dir(root: &Path, main_file: &str) -> Result<(Vec<u8>, usize), TypstError> {
    let main_path = root.join(main_file);
    if !main_path.exists() {
        return Err(TypstError::InvalidRequest {
            message: format!("main file not found: {}", main_path.display()),
        });
    }

    let engine = TypstEngine::builder()
        .with_file_system_resolver(root)
        .build();

    compile_engine(&engine, main_file)
}

fn compile_engine(engine: &TypstEngine, main_file: &str) -> Result<(Vec<u8>, usize), TypstError> {
    let result = engine.compile::<_, typst::layout::PagedDocument>(main_file);

    let document = result.output.map_err(|e| TypstError::CompilationError {
        message: format!("{e}"),
    })?;

    // Log warnings if any.
    for w in &result.warnings {
        tracing::warn!(severity = ?w.severity, "typst compilation warning: {:?}", w.message);
    }

    let page_count = document.pages.len();

    // Render PDF.
    let pdf_bytes =
        typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|diagnostics| {
            let messages: Vec<String> = diagnostics
                .iter()
                .map(|d| format!("{:?}: {:?}", d.severity, d.message))
                .collect();
            TypstError::CompilationError {
                message: format!("PDF generation failed: {}", messages.join("; ")),
            }
        })?;

    Ok((pdf_bytes, page_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_hello_world() {
        let mut files = HashMap::new();
        files.insert("main.typ".to_owned(), "Hello, World!".to_owned());

        let (pdf_bytes, page_count) = compile(&files, "main.typ").unwrap();
        assert!(!pdf_bytes.is_empty());
        assert!(page_count >= 1);
        // PDF magic bytes
        assert_eq!(&pdf_bytes[..5], b"%PDF-");
    }

    #[test]
    fn compile_missing_main_file() {
        let files = HashMap::new();
        let result = compile(&files, "main.typ");
        assert!(result.is_err());
    }
}
