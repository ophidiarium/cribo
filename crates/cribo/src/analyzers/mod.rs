//! Analyzers for processing collected data from AST visitors
//!
//! This module contains pure analysis logic separated from code generation.
//! Analyzers work with data collected by visitors to derive insights about
//! module dependencies, symbol relationships, and import requirements.

pub(crate) mod dependency_analyzer;
pub(crate) mod global_analyzer;
pub(crate) mod import_analyzer;
pub(crate) mod module_classifier;
pub(crate) mod symbol_analyzer;
pub(crate) mod types;

pub(crate) use global_analyzer::GlobalAnalyzer;
pub(crate) use import_analyzer::ImportAnalyzer;
pub(crate) use symbol_analyzer::SymbolAnalyzer;
