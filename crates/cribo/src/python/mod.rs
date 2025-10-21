//! Python-related helpers centralized here.
//!
//! This module owns constants and path/name utilities for Python module/package
//! discovery and classification. Other parts of the codebase should prefer
//! using these helpers instead of ad-hoc string checks.

pub(crate) mod constants;
pub(crate) mod module_path;
