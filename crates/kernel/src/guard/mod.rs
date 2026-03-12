//! Security guard system — taint tracking + pattern-based rule scanning.
//!
//! Prevents prompt injection attacks by:
//! 1. Tracking data provenance labels (taint) through the LLM context
//! 2. Scanning tool arguments for known dangerous patterns

pub mod pattern;
pub mod pipeline;
pub mod taint;
