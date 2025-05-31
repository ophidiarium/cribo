//! Serpen core library modules

pub mod resolver;
// you can also expose other modules if needed:
// pub mod bundler;
// pub mod emit;
// pub mod main; // if you factor out your CLI

pub mod bundler;
pub mod config;
pub mod dependency_graph;
pub mod emit;
pub mod python_stdlib;

pub use bundler::Bundler;
pub use config::Config;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pymodule]
fn serpen(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<bundler::PyBundler>()?;
    Ok(())
}
