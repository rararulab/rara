//! Compile-time embedded agent definitions.
//!
//! The bundled `.md` files under `defaults/` are included via
//! [`include_str!`] so they ship inside the binary without requiring
//! filesystem access at runtime.

use super::definition::AgentDefinition;

/// All bundled agent definitions, embedded at compile time.
pub fn all_bundled_agents() -> Vec<AgentDefinition> {
    let sources = [
        include_str!("defaults/scout.md"),
        include_str!("defaults/planner.md"),
        include_str!("defaults/worker.md"),
    ];
    sources
        .iter()
        .filter_map(|src| AgentDefinition::parse(src).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_agents_parse_successfully() {
        let agents = all_bundled_agents();
        assert_eq!(agents.len(), 3, "expected 3 bundled agents");
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"scout"));
        assert!(names.contains(&"planner"));
        assert!(names.contains(&"worker"));
    }
}
