//! Shared CLI context — opens the same stores the desktop app does.
//!
//! Construction is deliberately eager: every command needs at least the
//! config and one store, so opening them up-front keeps each command
//! body short and lets us surface a missing-data-dir or corrupt-DB
//! failure once at startup rather than command-by-command.
//!
//! Path resolution mirrors `lib.rs::try_setup` via the shared
//! `core::paths` helpers, so the CLI sees the same files as the
//! desktop app when both are run on the same machine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::activity::ActivityLog;
use crate::core::config::{load_or_default, AppConfig};
use crate::core::dimse::LogEmitter;
use crate::core::error::AppError;
use crate::core::paths::{self, DataPaths};
use crate::core::peers::PeerStore;
use crate::core::store::Index;
use crate::core::worklist::WorklistStore;

/// Opened-once handles every CLI subcommand needs.
pub struct Context {
    pub data_dir: PathBuf,
    pub paths: DataPaths,
    pub config: AppConfig,
    pub index: Arc<Index>,
    pub activity: Arc<ActivityLog>,
    pub peers: Arc<PeerStore>,
    pub worklist: Arc<WorklistStore>,
}

impl Context {
    /// Resolves the data directory (override or platform default),
    /// then opens every persistent store. Each file is created on
    /// demand if missing — running the CLI against a fresh directory
    /// is a supported usage.
    pub fn open(data_dir_override: Option<&Path>) -> Result<Self, AppError> {
        let data_dir = match data_dir_override {
            Some(p) => p.to_path_buf(),
            None => paths::default_data_dir()?,
        };
        std::fs::create_dir_all(&data_dir)?;

        let paths = paths::data_paths_from(&data_dir);

        // `default_with_home` only seeds an unwritten config. When the
        // user has never run the desktop app the file does not exist,
        // and we synthesise a default rooted at the home directory so
        // `config show` returns something usable.
        let home = home_dir().unwrap_or_else(|| data_dir.clone());
        let default = AppConfig::default_with_home(&home);
        let config = load_or_default(&paths.config, default)?;

        let index = Arc::new(Index::open(&paths.index)?);
        let activity = Arc::new(ActivityLog::open(&paths.index)?);
        let peers = Arc::new(PeerStore::open(&paths.peers)?);
        let worklist = Arc::new(WorklistStore::open(&paths.worklist)?);

        Ok(Self {
            data_dir,
            paths,
            config,
            index,
            activity,
            peers,
            worklist,
        })
    }

    /// Builds the persist-only activity emitter the SCU commands hand
    /// to `core::dimse`. Each event is recorded in `activity.sqlite`
    /// so subsequent `nightowl-cli activity list` calls see it.
    pub fn emitter(&self) -> LogEmitter {
        LogEmitter::new(self.activity.clone())
    }
}

fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}
