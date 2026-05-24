//! Configured remote DICOM peers (Application Entities).
//!
//! A `Peer` is a node Phantom can talk to: an AE Title plus the host
//! and port where it listens. The list is persisted as `peers.json` in
//! the Tauri app config directory next to `config.json` and `store.sqlite`.
//!
//! M6's C-MOVE handler resolves Move Destination AE Titles against
//! this list — that is why this lands before M6 even though the plan
//! orders Peers as M7.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::is_valid_ae_title;
use super::error::AppError;

/// A configured remote DICOM peer.
///
/// `id` is a UUID v4 assigned at creation so the UI can refer to a
/// peer stably even if the user renames it. `ae_title` is the DICOM
/// identifier other peers see when this entry is the target of an
/// outbound operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub ae_title: String,
    pub host: String,
    pub port: u16,
}

/// Fields a caller provides when creating a peer. `id` is assigned by
/// the store, never by the client.
#[derive(Debug, Clone, Deserialize)]
pub struct NewPeer {
    pub name: String,
    pub ae_title: String,
    pub host: String,
    pub port: u16,
}

/// Fields a caller provides when updating an existing peer.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePeer {
    pub id: String,
    pub name: String,
    pub ae_title: String,
    pub host: String,
    pub port: u16,
}

/// JSON-on-disk + in-memory `Peer` collection.
///
/// All mutating operations write the whole list back to disk atomically
/// (temp file + rename) so a crash mid-write cannot leave a truncated
/// `peers.json`. The peer count is small enough that rewriting the
/// whole file on every change is cheap.
pub struct PeerStore {
    path: PathBuf,
    state: Mutex<Vec<Peer>>,
}

impl PeerStore {
    /// Loads peers from `path`, or returns an empty store if the file
    /// does not exist yet. Any other failure (malformed JSON,
    /// permission error) propagates as `AppError`.
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let peers = if path.exists() {
            let bytes = std::fs::read(path)?;
            let parsed: Vec<Peer> = serde_json::from_slice(&bytes)?;
            parsed
        } else {
            Vec::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            state: Mutex::new(peers),
        })
    }

    pub fn list(&self) -> Result<Vec<Peer>, AppError> {
        Ok(self.lock()?.clone())
    }

    /// Validates the new peer fields, checks for AE-title duplicates,
    /// inserts, persists, and returns the assigned id.
    pub fn create(&self, new: NewPeer) -> Result<Peer, AppError> {
        validate_fields(&new.name, &new.ae_title, &new.host, new.port)?;
        let mut guard = self.lock()?;
        if guard.iter().any(|p| p.ae_title == new.ae_title) {
            return Err(AppError::validation(
                "ae_title",
                format!("AE Title {} is already in use by another peer", new.ae_title),
            ));
        }
        let peer = Peer {
            id: Uuid::new_v4().to_string(),
            name: new.name,
            ae_title: new.ae_title,
            host: new.host,
            port: new.port,
        };
        guard.push(peer.clone());
        write_atomic(&self.path, &*guard)?;
        Ok(peer)
    }

    /// Validates the updated fields, ensures no other peer claims the
    /// same AE Title, updates in place, persists.
    pub fn update(&self, update: UpdatePeer) -> Result<Peer, AppError> {
        validate_fields(&update.name, &update.ae_title, &update.host, update.port)?;
        let mut guard = self.lock()?;
        if guard
            .iter()
            .any(|p| p.id != update.id && p.ae_title == update.ae_title)
        {
            return Err(AppError::validation(
                "ae_title",
                format!("AE Title {} is already in use by another peer", update.ae_title),
            ));
        }
        let target = guard
            .iter_mut()
            .find(|p| p.id == update.id)
            .ok_or_else(|| {
                AppError::validation("id", format!("no peer with id {}", update.id))
            })?;
        target.name = update.name;
        target.ae_title = update.ae_title;
        target.host = update.host;
        target.port = update.port;
        let updated = target.clone();
        write_atomic(&self.path, &*guard)?;
        Ok(updated)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let mut guard = self.lock()?;
        let before = guard.len();
        guard.retain(|p| p.id != id);
        if guard.len() == before {
            return Err(AppError::validation("id", format!("no peer with id {id}")));
        }
        write_atomic(&self.path, &*guard)?;
        Ok(())
    }

    /// Looks up a peer by AE Title — what M6's C-MOVE handler does to
    /// resolve a Move Destination. AE Titles are case-sensitive per
    /// PS3.7 §D.3.3.3.
    pub fn find_by_ae_title(&self, ae_title: &str) -> Result<Option<Peer>, AppError> {
        Ok(self
            .lock()?
            .iter()
            .find(|p| p.ae_title == ae_title)
            .cloned())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Peer>>, AppError> {
        self.state
            .lock()
            .map_err(|_| AppError::Internal("peer store mutex poisoned".to_string()))
    }
}

fn validate_fields(name: &str, ae_title: &str, host: &str, port: u16) -> Result<(), AppError> {
    if name.trim().is_empty() {
        return Err(AppError::validation("name", "Name must not be empty."));
    }
    if !is_valid_ae_title(ae_title) {
        return Err(AppError::validation(
            "ae_title",
            "AE Title must be 1-16 printable ASCII characters with no leading or trailing whitespace.",
        ));
    }
    if host.trim().is_empty() {
        return Err(AppError::validation(
            "host",
            "Host must not be empty (hostname or IP address).",
        ));
    }
    if port == 0 {
        return Err(AppError::validation(
            "port",
            "Port must be between 1 and 65535.",
        ));
    }
    Ok(())
}

fn write_atomic(path: &Path, peers: &Vec<Peer>) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(peers)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (PathBuf, PeerStore) {
        let dir = std::env::temp_dir().join(format!(
            "phantom-peers-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("peers.json");
        let store = PeerStore::open(&path).expect("open");
        (path, store)
    }

    fn sample(name: &str, ae: &str, port: u16) -> NewPeer {
        NewPeer {
            name: name.to_string(),
            ae_title: ae.to_string(),
            host: "localhost".to_string(),
            port,
        }
    }

    #[test]
    fn create_then_list_returns_peer() {
        let (_path, store) = temp_store();
        let peer = store.create(sample("Test SCP", "TESTSCP", 11113)).unwrap();
        assert!(!peer.id.is_empty());
        assert_eq!(peer.ae_title, "TESTSCP");

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, peer.id);
    }

    #[test]
    fn create_duplicate_ae_title_rejected() {
        let (_path, store) = temp_store();
        store.create(sample("First", "DUP", 11113)).unwrap();
        let err = store
            .create(sample("Second", "DUP", 11114))
            .expect_err("duplicate should fail");
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn update_changes_fields_and_keeps_id() {
        let (_path, store) = temp_store();
        let peer = store.create(sample("Original", "ORIG", 11113)).unwrap();
        let updated = store
            .update(UpdatePeer {
                id: peer.id.clone(),
                name: "Renamed".to_string(),
                ae_title: "RENAMED".to_string(),
                host: "192.168.1.5".to_string(),
                port: 4242,
            })
            .unwrap();
        assert_eq!(updated.id, peer.id);
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.ae_title, "RENAMED");
        assert_eq!(updated.port, 4242);
    }

    #[test]
    fn update_unknown_id_rejected() {
        let (_path, store) = temp_store();
        let err = store
            .update(UpdatePeer {
                id: "nope".to_string(),
                name: "X".to_string(),
                ae_title: "X".to_string(),
                host: "localhost".to_string(),
                port: 1,
            })
            .expect_err("unknown id should fail");
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn delete_removes_peer() {
        let (_path, store) = temp_store();
        let peer = store.create(sample("To delete", "DEL", 11113)).unwrap();
        store.delete(&peer.id).unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn find_by_ae_title() {
        let (_path, store) = temp_store();
        store.create(sample("Alpha", "ALPHA", 11113)).unwrap();
        store.create(sample("Beta", "BETA", 11114)).unwrap();
        let found = store.find_by_ae_title("BETA").unwrap().expect("present");
        assert_eq!(found.name, "Beta");
        assert!(store.find_by_ae_title("MISSING").unwrap().is_none());
    }

    #[test]
    fn persists_across_reopen() {
        let (path, store) = temp_store();
        store.create(sample("Persist", "PERSIST", 11113)).unwrap();
        drop(store);
        let reopened = PeerStore::open(&path).unwrap();
        let listed = reopened.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ae_title, "PERSIST");
    }
}
