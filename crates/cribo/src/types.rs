//! Common type aliases used throughout the codebase

use std::hash::BuildHasherDefault;

use indexmap::{IndexMap, IndexSet};
use rustc_hash::FxHasher;

/// Type alias for `IndexMap` with `FxHasher` for better performance
pub type FxIndexMap<K, V> = IndexMap<K, V, BuildHasherDefault<FxHasher>>;

/// Type alias for `IndexSet` with `FxHasher` for better performance
pub type FxIndexSet<T> = IndexSet<T, BuildHasherDefault<FxHasher>>;
