//! Bundle compiler module for transforming analysis results into executable programs
//!
//! This module contains the compiler that transforms high-level analysis results
//! into low-level execution steps that can be run by the Bundle VM.

pub mod compiler;

pub use compiler::{BundleCompiler, BundleProgram, ExecutionStep};
