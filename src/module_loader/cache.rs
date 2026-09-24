// Module cache and circular dependency detection engine.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use crate::bytecode::proto::Proto;
use crate::vm::value::Value;

#[derive(Clone, Debug, Default)]
pub struct ModuleCache {
    loaded_values: HashMap<String, Value>,
    compiled_protos: HashMap<PathBuf, Rc<Proto>>,
    loading_stack: Vec<String>,
    loading_set: HashSet<String>,
}

impl ModuleCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_value(&self, key: &str) -> Option<Value> {
        self.loaded_values.get(key).cloned()
    }

    pub fn insert_value(&mut self, key: impl Into<String>, value: Value) {
        self.loaded_values.insert(key.into(), value);
    }

    pub fn get_proto(&self, path: &PathBuf) -> Option<Rc<Proto>> {
        self.compiled_protos.get(path).cloned()
    }

    pub fn insert_proto(&mut self, path: PathBuf, proto: Rc<Proto>) {
        self.compiled_protos.insert(path, proto);
    }

    pub fn begin_loading(&mut self, key: &str) -> Result<(), String> {
        if self.loading_set.contains(key) {
            let cycle = self.loading_stack.join(" -> ");
            return Err(format!(
                "circular dependency detected while requiring '{}': {} -> {}",
                key, cycle, key
            ));
        }
        self.loading_set.insert(key.to_string());
        self.loading_stack.push(key.to_string());
        Ok(())
    }

    pub fn finish_loading(&mut self, key: &str) {
        self.loading_set.remove(key);
        if let Some(pos) = self.loading_stack.iter().rposition(|s| s == key) {
            self.loading_stack.remove(pos);
        }
    }
}
