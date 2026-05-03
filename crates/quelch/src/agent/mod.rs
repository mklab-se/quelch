//! Agent and skill bundle generation (`quelch agent generate`).
//!
//! Builds a [`Bundle`] from a deployment's config and then writes it in the
//! format appropriate for the chosen target platform.
//!
//! # Usage
//!
//! ```no_run
//! use quelch::agent::bundle;
//! use quelch::agent::targets::claude_code;
//! use quelch::config::load_config;
//! use std::path::Path;
//!
//! let config = load_config(Path::new("quelch.yaml")).unwrap();
//! let bndl = bundle::build(&config, "mcp").unwrap();
//! claude_code::write(&bndl, Path::new("./agent-bundle")).unwrap();
//! ```

pub mod bundle;
pub mod error;
pub mod targets;
