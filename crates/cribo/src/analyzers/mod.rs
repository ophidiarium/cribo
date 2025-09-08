//! Analyzers for processing collected data from AST visitors
//!
//! This module contains pure analysis logic separated from code generation.
//! Analyzers work with data collected by visitors to derive insights about
//! module dependencies, symbol relationships, and import requirements.

pub mod dependency_analyzer;
pub mod global_analyzer;
pub mod import_analyzer;
pub mod module_classifier;
pub mod symbol_analyzer;
pub mod types;

pub use global_analyzer::GlobalAnalyzer;
pub use import_analyzer::ImportAnalyzer;
pub use module_classifier::ModuleClassifier;
pub use symbol_analyzer::SymbolAnalyzer;
