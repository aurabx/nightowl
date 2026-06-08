//! Subcommand module tree. Each submodule defines a clap `Action`
//! enum plus a `run()` dispatcher invoked from `main.rs`.

pub mod activity;
pub mod config;
pub mod instances;
pub mod peers;
pub mod scu;
pub mod series;
pub mod store;
pub mod studies;
pub mod worklist;
