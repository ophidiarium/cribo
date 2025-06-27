//! Shared type definitions for the cribo crate
//!
//! This module contains common types that are used across multiple components
//! of the bundler, ensuring consistency and avoiding circular dependencies.

/// Classification of a module based on its origin
///
/// This enum represents the fundamental categorization of Python modules,
/// which is used throughout the bundling process for making decisions about
/// hoisting, inlining, and dependency management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleKind {
    /// Python standard library modules (e.g., os, sys, json)
    StandardLibrary,

    /// Third-party packages installed via pip/conda (e.g., numpy, requests)
    ThirdParty,

    /// First-party modules that are part of the project being bundled
    FirstParty,
}

impl ModuleKind {
    /// Check if this is a standard library module
    pub fn is_stdlib(&self) -> bool {
        matches!(self, ModuleKind::StandardLibrary)
    }

    /// Check if this is a third-party module
    pub fn is_third_party(&self) -> bool {
        matches!(self, ModuleKind::ThirdParty)
    }

    /// Check if this is a first-party module
    pub fn is_first_party(&self) -> bool {
        matches!(self, ModuleKind::FirstParty)
    }
}

impl std::fmt::Display for ModuleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleKind::StandardLibrary => write!(f, "stdlib"),
            ModuleKind::ThirdParty => write!(f, "third-party"),
            ModuleKind::FirstParty => write!(f, "first-party"),
        }
    }
}
