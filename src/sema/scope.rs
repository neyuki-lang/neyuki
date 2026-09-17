// Lexical scoping and scope stack manager for semantic analysis.

use std::collections::HashMap;

use crate::diagnostics::span::Span;
use crate::sema::symbol::Symbol;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Global,
    Function,
    Block,
    Loop,
}

pub struct Scope {
    pub kind: ScopeKind,
    pub symbols: HashMap<String, Symbol>,
}

impl Scope {
    pub fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            symbols: HashMap::new(),
        }
    }
}

pub struct ScopeManager {
    pub scopes: Vec<Scope>,
}

impl Default for ScopeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeManager {
    pub fn new() -> Self {
        let mut manager = Self { scopes: Vec::new() };
        manager.enter_scope(ScopeKind::Global);
        manager
    }

    pub fn enter_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope::new(kind));
    }

    pub fn exit_scope(&mut self) -> Vec<Symbol> {
        if self.scopes.len() > 1 {
            let scope = self.scopes.pop().unwrap();
            scope.symbols.into_values().collect()
        } else {
            Vec::new()
        }
    }

    pub fn define(&mut self, symbol: Symbol) -> Result<(), Span> {
        let current = self.scopes.last_mut().expect("scope stack cannot be empty");
        if let Some(existing) = current.symbols.get(&symbol.name)
            && existing.is_const
        {
            return Err(existing.span);
        }
        current.symbols.insert(symbol.name.clone(), symbol);
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.symbols.get(name) {
                return Some(sym);
            }
        }
        None
    }

    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut Symbol> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(sym) = scope.symbols.get_mut(name) {
                return Some(sym);
            }
        }
        None
    }

    pub fn flush_all(&mut self) -> Vec<Symbol> {
        let mut all = Vec::new();
        while let Some(scope) = self.scopes.pop() {
            all.extend(scope.symbols.into_values());
        }
        all
    }

    // Check if symbol exists in an enclosing outer scope
    pub fn find_outer(&self, name: &str) -> Option<&Symbol> {
        if self.scopes.len() <= 1 {
            return None;
        }
        // Exclude the current innermost scope
        for scope in self.scopes[0..self.scopes.len() - 1].iter().rev() {
            if let Some(sym) = scope.symbols.get(name) {
                return Some(sym);
            }
        }
        None
    }

    pub fn mark_used(&mut self, name: &str) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(sym) = scope.symbols.get_mut(name) {
                sym.used = true;
                return true;
            }
        }
        false
    }
}
