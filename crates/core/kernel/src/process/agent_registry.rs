use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use dashmap::DashMap;

use super::AgentManifest;
use crate::error::{KernelError, Result};

pub struct AgentRegistry {
    builtin: HashMap<String, AgentManifest>,
    custom: DashMap<String, AgentManifest>,
    agents_dir: PathBuf,
}

impl AgentRegistry {
    pub fn new(builtin: Vec<AgentManifest>, agents_dir: PathBuf) -> Self {
        let builtin = builtin
            .into_iter()
            .map(|m| (m.name.clone(), m))
            .collect();
        Self {
            builtin,
            custom: DashMap::new(),
            agents_dir,
        }
    }

    pub fn init(
        builtin: Vec<AgentManifest>,
        loader: &super::manifest_loader::ManifestLoader,
        agents_dir: PathBuf,
    ) -> Self {
        let registry = Self::new(builtin, agents_dir);
        for manifest in loader.list() {
            let name = manifest.name.clone();
            // Only add to custom if not already a builtin
            if !registry.builtin.contains_key(&name) {
                registry.custom.insert(name, manifest.clone());
            }
        }
        registry
    }

    pub fn get(&self, name: &str) -> Option<AgentManifest> {
        // Custom first (shadow), then builtin
        if let Some(m) = self.custom.get(name) {
            return Some(m.value().clone());
        }
        self.builtin.get(name).cloned()
    }

    pub fn list(&self) -> Vec<AgentManifest> {
        let mut result: HashMap<String, AgentManifest> = self.builtin.clone();
        for entry in self.custom.iter() {
            result.insert(entry.key().clone(), entry.value().clone());
        }
        result.into_values().collect()
    }

    pub fn register(&self, manifest: AgentManifest) -> Result<()> {
        let name = manifest.name.clone();
        // Persist to YAML
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let yaml = serde_yaml::to_string(&manifest).map_err(|e| KernelError::Other {
            message: format!("failed to serialize manifest: {e}").into(),
        })?;
        std::fs::write(&path, yaml).map_err(|e| KernelError::IO {
            source: e,
            location: snafu::Location::new(file!(), line!(), 0),
        })?;
        self.custom.insert(name, manifest);
        Ok(())
    }

    pub fn unregister(&self, name: &str) -> Result<()> {
        if self.builtin.contains_key(name) {
            return Err(KernelError::Other {
                message: format!("cannot unregister builtin agent: {name}").into(),
            });
        }
        self.custom.remove(name);
        let path = self.agents_dir.join(format!("{}.yaml", name));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtin.contains_key(name)
    }

    pub fn agents_dir(&self) -> &Path {
        &self.agents_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Priority;

    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            role: None,
            description: format!("Test agent: {name}"),
            model: "test-model".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            soul_prompt: None,
            provider_hint: None,
            max_iterations: Some(10),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        }
    }

    #[test]
    fn test_get_builtin() {
        let registry = AgentRegistry::new(
            vec![test_manifest("rara")],
            std::env::temp_dir().join("agent_registry_test_get"),
        );
        assert!(registry.get("rara").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_custom_shadows_builtin() {
        let registry = AgentRegistry::new(
            vec![test_manifest("rara")],
            std::env::temp_dir().join("agent_registry_test_shadow"),
        );
        let mut custom = test_manifest("rara");
        custom.model = "custom-model".to_string();
        registry.custom.insert("rara".to_string(), custom);

        let m = registry.get("rara").unwrap();
        assert_eq!(m.model, "custom-model");
    }

    #[test]
    fn test_list_combines_builtin_and_custom() {
        let registry = AgentRegistry::new(
            vec![test_manifest("rara")],
            std::env::temp_dir().join("agent_registry_test_list"),
        );
        registry.custom.insert("scout".to_string(), test_manifest("scout"));

        let all = registry.list();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|m| m.name == "rara"));
        assert!(all.iter().any(|m| m.name == "scout"));
    }

    #[test]
    fn test_register_and_persist() {
        let dir = std::env::temp_dir().join("agent_registry_test_register");
        let _ = std::fs::remove_dir_all(&dir);

        let registry = AgentRegistry::new(vec![], dir.clone());
        registry.register(test_manifest("new-agent")).unwrap();

        assert!(registry.get("new-agent").is_some());
        assert!(dir.join("new-agent.yaml").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unregister_custom() {
        let dir = std::env::temp_dir().join("agent_registry_test_unregister");
        let _ = std::fs::remove_dir_all(&dir);

        let registry = AgentRegistry::new(vec![], dir.clone());
        registry.register(test_manifest("removable")).unwrap();
        assert!(registry.get("removable").is_some());

        registry.unregister("removable").unwrap();
        assert!(registry.get("removable").is_none());
        assert!(!dir.join("removable.yaml").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unregister_builtin_fails() {
        let registry = AgentRegistry::new(
            vec![test_manifest("rara")],
            std::env::temp_dir().join("agent_registry_test_builtin_fail"),
        );
        let result = registry.unregister("rara");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("builtin"));
    }

    #[test]
    fn test_init_from_loader() {
        let mut loader = super::super::manifest_loader::ManifestLoader::new();
        loader.load_manifests(vec![test_manifest("from-loader")]);

        let registry = AgentRegistry::init(
            vec![test_manifest("builtin")],
            &loader,
            std::env::temp_dir().join("agent_registry_test_init"),
        );

        assert!(registry.get("builtin").is_some());
        assert!(registry.get("from-loader").is_some());
        assert_eq!(registry.list().len(), 2);
    }
}
