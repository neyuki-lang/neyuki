// Unique node identifier for AST nodes in Neyuki compiler.

use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

static GLOBAL_NODE_COUNTER: AtomicU32 = AtomicU32::new(1);

impl NodeId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn next() -> Self {
        let id = GLOBAL_NODE_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(id)
    }

    pub fn is_dummy(&self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[derive(Debug, Default)]
pub struct NodeIdGenerator {
    current: u32,
}

impl NodeIdGenerator {
    pub fn new() -> Self {
        Self { current: 1 }
    }

    pub fn next_id(&mut self) -> NodeId {
        let id = self.current;
        self.current = self.current.saturating_add(1);
        NodeId(id)
    }

    pub fn reset(&mut self) {
        self.current = 1;
    }
}
