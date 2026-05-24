//! Local SOP Instance index.
//!
//! Phantom treats a configured directory on disk as a tiny PACS. This
//! module owns the SQLite index that mirrors that directory so the M4
//! C-FIND SCP can answer queries without re-parsing every file, and so
//! the Store page can show the Patient → Study → Series → SOP Instance
//! hierarchy without a filesystem walk on every render.
//!
//! Vocabulary:
//! - **SOP Instance**: one DICOM file (one image, one report, one
//!   encapsulated PDF, etc.). The fundamental unit of storage.
//! - **Series**: a group of SOP Instances acquired together (typically
//!   one scan).
//! - **Study**: a group of Series acquired during one patient visit.
//! - **Transfer Syntax**: the wire encoding for the DICOM data set
//!   (Implicit VR LE, Explicit VR LE, JPEG, …). Captured here so the
//!   SCP can answer with the negotiated syntax later.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::{open_file, DefaultDicomObject};
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::error::AppError;

/// Schema applied at `Index::open`. Idempotent via `IF NOT EXISTS`.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sop_instances (
    sop_instance_uid     TEXT PRIMARY KEY,
    series_instance_uid  TEXT NOT NULL,
    study_instance_uid   TEXT NOT NULL,
    patient_id           TEXT NOT NULL,
    patient_name         TEXT,
    study_description    TEXT,
    series_description   TEXT,
    modality             TEXT,
    study_date           TEXT,
    sop_class_uid        TEXT NOT NULL,
    transfer_syntax_uid  TEXT NOT NULL,
    file_path            TEXT NOT NULL UNIQUE,
    size_bytes           INTEGER NOT NULL,
    ingested_at          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_study   ON sop_instances(study_instance_uid);
CREATE INDEX IF NOT EXISTS idx_series  ON sop_instances(series_instance_uid);
CREATE INDEX IF NOT EXISTS idx_patient ON sop_instances(patient_id);
";

// ---------------------------------------------------------------------
// Public data shapes
// ---------------------------------------------------------------------

/// Outcome of a single-file ingestion. Reported back by `ingest_file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail")]
pub enum IngestOutcome {
    /// File parsed cleanly and a new index row was created.
    Inserted,
    /// File parsed cleanly and replaced an existing row with the same
    /// `sop_instance_uid` (the on-disk file may have moved or been
    /// rewritten).
    Updated,
    /// File deliberately skipped — not a DICOM file, missing required
    /// tags, etc. The `reason` is shown in the activity log.
    Skipped { reason: String },
}

/// Aggregate result of a directory rescan.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub files_seen: u64,
    pub files_inserted: u64,
    pub files_updated: u64,
    pub files_skipped: u64,
    pub files_errored: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyRow {
    pub study_instance_uid: String,
    pub patient_id: String,
    pub patient_name: Option<String>,
    pub study_description: Option<String>,
    pub study_date: Option<String>,
    /// Comma-separated distinct modalities present in the study.
    pub modalities: Option<String>,
    pub series_count: i64,
    pub instance_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesRow {
    pub series_instance_uid: String,
    pub study_instance_uid: String,
    pub series_description: Option<String>,
    pub modality: Option<String>,
    pub instance_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRow {
    pub sop_instance_uid: String,
    pub series_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
    pub file_path: String,
    pub size_bytes: i64,
}

// ---------------------------------------------------------------------
// C-FIND query types (M4)
// ---------------------------------------------------------------------

/// DICOM Query/Retrieve hierarchy level used for C-FIND, C-MOVE and
/// C-GET. PS3.4 Annex C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindLevel {
    Patient,
    Study,
    Series,
    Image,
}

/// A single matching key, translated from the inbound DICOM identifier.
///
/// The DICOM matching key types we support at M4 are: Single Value
/// (exact equality), Wildcard (`*` matches any, `?` matches one),
/// List of UID (backslash-separated values), and Range (two dates
/// joined by a hyphen). Universal Matching — an empty value meaning
/// "return this key, do not filter on it" — is represented by the
/// absence of a `KeyMatch` rather than a variant.
#[derive(Debug, Clone)]
pub enum KeyMatch {
    /// Exact equality.
    Single(String),
    /// DICOM wildcard pattern; `*` and `?` translate to SQL `%` and
    /// `_` after escaping any literal SQL wildcards in the input.
    Wildcard(String),
    /// Backslash-separated list of UIDs; SQL `IN (?, ?, …)`.
    List(Vec<String>),
    /// `YYYYMMDD-YYYYMMDD` (either side optional in DICOM but here
    /// both bounds are required after splitting).
    Range(String, String),
}

/// Translated representation of a C-FIND-RQ identifier dataset, ready
/// to feed to `Index::find`.
///
/// Every field is optional. A `None` value means the key was either
/// absent from the request or present with an empty value (Universal
/// Matching). Both cases mean "do not filter on this key" — the
/// response identifier will still carry the column populated.
#[derive(Debug, Clone)]
pub struct FindQuery {
    pub level: FindLevel,
    pub patient_id: Option<KeyMatch>,
    pub patient_name: Option<KeyMatch>,
    pub study_instance_uid: Option<KeyMatch>,
    pub study_date: Option<KeyMatch>,
    pub modality: Option<KeyMatch>,
    pub series_instance_uid: Option<KeyMatch>,
    pub sop_instance_uid: Option<KeyMatch>,
    pub sop_class_uid: Option<KeyMatch>,
}

impl FindQuery {
    pub fn new(level: FindLevel) -> Self {
        Self {
            level,
            patient_id: None,
            patient_name: None,
            study_instance_uid: None,
            study_date: None,
            modality: None,
            series_instance_uid: None,
            sop_instance_uid: None,
            sop_class_uid: None,
        }
    }
}

/// One match returned from `Index::find`. Fields irrelevant to the
/// queried level are `None`. The dimse code builds the response
/// identifier from this row, populating the keys the client asked for.
#[derive(Debug, Default, Clone)]
pub struct FindRow {
    pub patient_id: String,
    pub patient_name: Option<String>,
    pub study_instance_uid: Option<String>,
    pub study_description: Option<String>,
    pub study_date: Option<String>,
    /// Comma-separated distinct modalities present in the study.
    pub modalities_in_study: Option<String>,
    pub number_of_study_related_series: Option<i64>,
    pub number_of_study_related_instances: Option<i64>,
    pub series_instance_uid: Option<String>,
    pub series_description: Option<String>,
    pub modality: Option<String>,
    pub number_of_series_related_instances: Option<i64>,
    pub sop_instance_uid: Option<String>,
    pub sop_class_uid: Option<String>,
}

/// One SOP Instance enough for `forward_via_c_store` to forward it.
#[derive(Debug, Clone)]
pub struct RetrieveInstance {
    pub sop_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
    pub file_path: String,
}

/// All tags extracted from a DICOM file, ready to insert.
#[derive(Debug, Clone)]
struct ParsedInstance {
    sop_instance_uid: String,
    series_instance_uid: String,
    study_instance_uid: String,
    patient_id: String,
    patient_name: Option<String>,
    study_description: Option<String>,
    series_description: Option<String>,
    modality: Option<String>,
    study_date: Option<String>,
    sop_class_uid: String,
    transfer_syntax_uid: String,
    file_path: PathBuf,
    size_bytes: i64,
}

// ---------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------

/// SQLite-backed SOP Instance index. Cheap to clone via `Arc`.
///
/// The connection sits behind a mutex because rusqlite's `Connection` is
/// `!Send + !Sync` and we want to share it across Tauri command threads.
/// For our access pattern (one writer at a time, infrequent reads from
/// the UI) the mutex is not a meaningful bottleneck.
pub struct Index {
    conn: Mutex<Connection>,
}

impl Index {
    /// Opens (or creates) a SQLite database at `path` and applies the
    /// schema. Creates the parent directory if missing.
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        // Pragmas: WAL gives us readers-don't-block-writers; synchronous
        // NORMAL is the WAL recommended level (fully durable at
        // checkpoint, sufficient for a dev tool).
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Parses one file and inserts-or-replaces its row.
    pub fn ingest_file(&self, file_path: &Path) -> Result<IngestOutcome, AppError> {
        let parsed = match parse_dicom(file_path) {
            Ok(p) => p,
            Err(reason) => {
                return Ok(IngestOutcome::Skipped { reason });
            }
        };

        let conn = self.lock()?;
        // Detect insert vs update so the activity log can distinguish.
        let existed: bool = conn
            .query_row(
                "SELECT 1 FROM sop_instances WHERE sop_instance_uid = ?1",
                params![parsed.sop_instance_uid],
                |_| Ok(true),
            )
            .unwrap_or(false);

        conn.execute(
            "INSERT OR REPLACE INTO sop_instances (
                sop_instance_uid, series_instance_uid, study_instance_uid,
                patient_id, patient_name, study_description, series_description,
                modality, study_date, sop_class_uid, transfer_syntax_uid,
                file_path, size_bytes, ingested_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                parsed.sop_instance_uid,
                parsed.series_instance_uid,
                parsed.study_instance_uid,
                parsed.patient_id,
                parsed.patient_name,
                parsed.study_description,
                parsed.series_description,
                parsed.modality,
                parsed.study_date,
                parsed.sop_class_uid,
                parsed.transfer_syntax_uid,
                parsed.file_path.to_string_lossy(),
                parsed.size_bytes,
                now_unix_ms() as i64,
            ],
        )?;

        Ok(if existed {
            IngestOutcome::Updated
        } else {
            IngestOutcome::Inserted
        })
    }

    /// Walks `dir` recursively and ingests every regular file.
    ///
    /// Non-DICOM files, files missing required tags, or files with
    /// unreadable bytes are reported as `Skipped` in the returned
    /// `ScanReport`. IO errors during traversal count as `errored`.
    pub fn rescan_dir(&self, dir: &Path) -> Result<ScanReport, AppError> {
        let start = std::time::Instant::now();
        let mut report = ScanReport::default();

        for entry in WalkDir::new(dir).follow_links(false) {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(error = %err, "scan: traversal error");
                    report.files_errored += 1;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            report.files_seen += 1;
            match self.ingest_file(entry.path()) {
                Ok(IngestOutcome::Inserted) => report.files_inserted += 1,
                Ok(IngestOutcome::Updated) => report.files_updated += 1,
                Ok(IngestOutcome::Skipped { reason }) => {
                    tracing::debug!(path = %entry.path().display(), %reason, "scan: skipped");
                    report.files_skipped += 1;
                }
                Err(err) => {
                    tracing::warn!(path = %entry.path().display(), error = %err, "scan: ingest failed");
                    report.files_errored += 1;
                }
            }
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// Returns all studies in the index, grouped and ordered most-recent
    /// `study_date` first.
    pub fn list_studies(&self) -> Result<Vec<StudyRow>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT
                study_instance_uid,
                MIN(patient_id),
                MIN(patient_name),
                MIN(study_description),
                MIN(study_date),
                GROUP_CONCAT(DISTINCT modality),
                COUNT(DISTINCT series_instance_uid),
                COUNT(*)
             FROM sop_instances
             GROUP BY study_instance_uid
             ORDER BY MIN(study_date) DESC NULLS LAST, MIN(patient_name)",
        )?;

        let rows = stmt
            .query_map([], |row| {
                Ok(StudyRow {
                    study_instance_uid: row.get(0)?,
                    patient_id: row.get(1)?,
                    patient_name: row.get(2)?,
                    study_description: row.get(3)?,
                    study_date: row.get(4)?,
                    modalities: row.get(5)?,
                    series_count: row.get(6)?,
                    instance_count: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_series_for_study(&self, study_uid: &str) -> Result<Vec<SeriesRow>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT
                series_instance_uid,
                MIN(study_instance_uid),
                MIN(series_description),
                MIN(modality),
                COUNT(*)
             FROM sop_instances
             WHERE study_instance_uid = ?1
             GROUP BY series_instance_uid
             ORDER BY MIN(series_description), series_instance_uid",
        )?;

        let rows = stmt
            .query_map(params![study_uid], map_series_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_instances_for_series(
        &self,
        series_uid: &str,
    ) -> Result<Vec<InstanceRow>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT sop_instance_uid, series_instance_uid, sop_class_uid,
                    transfer_syntax_uid, file_path, size_bytes
             FROM sop_instances
             WHERE series_instance_uid = ?1
             ORDER BY sop_instance_uid",
        )?;

        let rows = stmt
            .query_map(params![series_uid], map_instance_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns the total SOP Instance count, for the Store header.
    pub fn total_instance_count(&self) -> Result<i64, AppError> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sop_instances", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Runs a DICOM C-FIND query against the SOP Instance index.
    ///
    /// The returned `Vec<FindRow>` has one entry per match at the
    /// requested level. The dimse layer is responsible for converting
    /// each row into a C-FIND-RSP identifier dataset by populating the
    /// keys the client asked for.
    pub fn find(&self, q: &FindQuery) -> Result<Vec<FindRow>, AppError> {
        match q.level {
            FindLevel::Patient => self.find_patients(q),
            FindLevel::Study => self.find_studies_qr(q),
            FindLevel::Series => self.find_series_qr(q),
            FindLevel::Image => self.find_instances_qr(q),
        }
    }

    fn find_patients(&self, q: &FindQuery) -> Result<Vec<FindRow>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);

        let mut sql = String::from(
            "SELECT patient_id, MIN(patient_name) FROM sop_instances",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(" GROUP BY patient_id ORDER BY MIN(patient_name)");

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(FindRow {
                    patient_id: row.get(0)?,
                    patient_name: row.get(1)?,
                    ..FindRow::default()
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn find_studies_qr(&self, q: &FindQuery) -> Result<Vec<FindRow>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_instance_uid", q.study_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_date", q.study_date.as_ref(), &mut where_parts, &mut bound);

        let mut sql = String::from(
            "SELECT
                study_instance_uid,
                MIN(patient_id),
                MIN(patient_name),
                MIN(study_description),
                MIN(study_date),
                GROUP_CONCAT(DISTINCT modality),
                COUNT(DISTINCT series_instance_uid),
                COUNT(*)
             FROM sop_instances",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(" GROUP BY study_instance_uid ORDER BY MIN(study_date) DESC");

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(FindRow {
                    study_instance_uid: Some(row.get(0)?),
                    patient_id: row.get(1)?,
                    patient_name: row.get(2)?,
                    study_description: row.get(3)?,
                    study_date: row.get(4)?,
                    modalities_in_study: row.get(5)?,
                    number_of_study_related_series: Some(row.get(6)?),
                    number_of_study_related_instances: Some(row.get(7)?),
                    ..FindRow::default()
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn find_series_qr(&self, q: &FindQuery) -> Result<Vec<FindRow>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_instance_uid", q.study_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("series_instance_uid", q.series_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("modality", q.modality.as_ref(), &mut where_parts, &mut bound);

        let mut sql = String::from(
            "SELECT
                series_instance_uid,
                MIN(study_instance_uid),
                MIN(patient_id),
                MIN(patient_name),
                MIN(series_description),
                MIN(modality),
                COUNT(*)
             FROM sop_instances",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(" GROUP BY series_instance_uid ORDER BY MIN(series_description), series_instance_uid");

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(FindRow {
                    series_instance_uid: Some(row.get(0)?),
                    study_instance_uid: Some(row.get(1)?),
                    patient_id: row.get(2)?,
                    patient_name: row.get(3)?,
                    series_description: row.get(4)?,
                    modality: row.get(5)?,
                    number_of_series_related_instances: Some(row.get(6)?),
                    ..FindRow::default()
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns one row per matching SOP Instance regardless of the
    /// query level, with everything `forward_via_c_store` needs to
    /// forward each instance: the SOP Class UID, the on-disk transfer
    /// syntax, and the file path.
    ///
    /// At PATIENT / STUDY / SERIES levels this expands the match: a
    /// STUDY-level query for `PatientID=12345` returns every SOP
    /// Instance under every matching study. At IMAGE level the same
    /// keys filter individual instances.
    pub fn resolve_for_retrieve(
        &self,
        q: &FindQuery,
    ) -> Result<Vec<RetrieveInstance>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_instance_uid", q.study_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_date", q.study_date.as_ref(), &mut where_parts, &mut bound);
        apply_match("series_instance_uid", q.series_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("modality", q.modality.as_ref(), &mut where_parts, &mut bound);
        apply_match("sop_instance_uid", q.sop_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("sop_class_uid", q.sop_class_uid.as_ref(), &mut where_parts, &mut bound);

        let mut sql = String::from(
            "SELECT sop_instance_uid, sop_class_uid, transfer_syntax_uid, file_path
             FROM sop_instances",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(" ORDER BY study_instance_uid, series_instance_uid, sop_instance_uid");

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(RetrieveInstance {
                    sop_instance_uid: row.get(0)?,
                    sop_class_uid: row.get(1)?,
                    transfer_syntax_uid: row.get(2)?,
                    file_path: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn find_instances_qr(&self, q: &FindQuery) -> Result<Vec<FindRow>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);
        apply_match("study_instance_uid", q.study_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("series_instance_uid", q.series_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("sop_instance_uid", q.sop_instance_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("sop_class_uid", q.sop_class_uid.as_ref(), &mut where_parts, &mut bound);
        apply_match("modality", q.modality.as_ref(), &mut where_parts, &mut bound);

        let mut sql = String::from(
            "SELECT
                sop_instance_uid,
                series_instance_uid,
                study_instance_uid,
                patient_id,
                patient_name,
                sop_class_uid,
                modality
             FROM sop_instances",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(" ORDER BY sop_instance_uid");

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(FindRow {
                    sop_instance_uid: Some(row.get(0)?),
                    series_instance_uid: Some(row.get(1)?),
                    study_instance_uid: Some(row.get(2)?),
                    patient_id: row.get(3)?,
                    patient_name: row.get(4)?,
                    sop_class_uid: Some(row.get(5)?),
                    modality: row.get(6)?,
                    ..FindRow::default()
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
        self.conn
            .lock()
            .map_err(|_| AppError::Internal("store index mutex poisoned".to_string()))
    }
}

// ---------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------

fn map_series_row(row: &Row<'_>) -> rusqlite::Result<SeriesRow> {
    Ok(SeriesRow {
        series_instance_uid: row.get(0)?,
        study_instance_uid: row.get(1)?,
        series_description: row.get(2)?,
        modality: row.get(3)?,
        instance_count: row.get(4)?,
    })
}

fn map_instance_row(row: &Row<'_>) -> rusqlite::Result<InstanceRow> {
    Ok(InstanceRow {
        sop_instance_uid: row.get(0)?,
        series_instance_uid: row.get(1)?,
        sop_class_uid: row.get(2)?,
        transfer_syntax_uid: row.get(3)?,
        file_path: row.get(4)?,
        size_bytes: row.get(5)?,
    })
}

// ---------------------------------------------------------------------
// DICOM parsing
// ---------------------------------------------------------------------

/// Reads a DICOM file and extracts the tags we index.
///
/// Returns `Err(reason)` (a human-readable string) on any failure that
/// should be reported to the user as "skipped" rather than as an
/// internal error. Errors that genuinely indicate broken infrastructure
/// (out of memory, I/O subsystem failure) propagate via `AppError`
/// elsewhere; here we treat read failure as "not a DICOM file we can
/// index" and move on.
fn parse_dicom(path: &Path) -> Result<ParsedInstance, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("stat failed: {e}"))?;

    let obj: DefaultDicomObject =
        open_file(path).map_err(|e| format!("not a parseable DICOM file: {e}"))?;

    let transfer_syntax_uid = obj.meta().transfer_syntax.clone();

    let sop_instance_uid = req_str(&obj, tags::SOP_INSTANCE_UID, "SOPInstanceUID")?;
    let series_instance_uid = req_str(&obj, tags::SERIES_INSTANCE_UID, "SeriesInstanceUID")?;
    let study_instance_uid = req_str(&obj, tags::STUDY_INSTANCE_UID, "StudyInstanceUID")?;
    let sop_class_uid = req_str(&obj, tags::SOP_CLASS_UID, "SOPClassUID")?;

    // PatientID is DICOM Type 2 — required but may be empty. Treat
    // missing as empty rather than rejecting the file.
    let patient_id = opt_str(&obj, tags::PATIENT_ID).unwrap_or_default();

    Ok(ParsedInstance {
        sop_instance_uid,
        series_instance_uid,
        study_instance_uid,
        patient_id,
        patient_name: opt_str(&obj, tags::PATIENT_NAME),
        study_description: opt_str(&obj, tags::STUDY_DESCRIPTION),
        series_description: opt_str(&obj, tags::SERIES_DESCRIPTION),
        modality: opt_str(&obj, tags::MODALITY),
        study_date: opt_str(&obj, tags::STUDY_DATE),
        sop_class_uid,
        transfer_syntax_uid,
        file_path: path.to_path_buf(),
        size_bytes: metadata.len() as i64,
    })
}

fn req_str(obj: &DefaultDicomObject, tag: Tag, name: &str) -> Result<String, String> {
    let element = obj
        .element(tag)
        .map_err(|e| format!("missing required {name}: {e}"))?;
    let value = element
        .to_str()
        .map_err(|e| format!("invalid value for {name}: {e}"))?;
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        return Err(format!("empty required {name}"));
    }
    Ok(trimmed)
}

fn opt_str(obj: &DefaultDicomObject, tag: Tag) -> Option<String> {
    let element = obj.element(tag).ok()?;
    let value = element.to_str().ok()?;
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Matching-key → SQL translation (M4 C-FIND)
// ---------------------------------------------------------------------

/// Appends a WHERE clause and binding(s) for one matching key.
///
/// Returns silently when `m` is `None` (Universal Matching — the key
/// is returned but not filtered on).
fn apply_match(
    column: &str,
    m: Option<&KeyMatch>,
    where_parts: &mut Vec<String>,
    bound: &mut Vec<Box<dyn rusqlite::ToSql>>,
) {
    let Some(m) = m else { return };
    match m {
        KeyMatch::Single(v) => {
            where_parts.push(format!("{column} = ?"));
            bound.push(Box::new(v.clone()));
        }
        KeyMatch::Wildcard(pattern) => {
            // Two-stage translation so the user can search for a
            // literal underscore by typing it. First escape SQL LIKE
            // metacharacters in the source pattern, THEN map DICOM
            // wildcards to their SQL equivalents.
            let escaped = pattern
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like = escaped.replace('*', "%").replace('?', "_");
            where_parts.push(format!("{column} LIKE ? ESCAPE '\\'"));
            bound.push(Box::new(like));
        }
        KeyMatch::List(values) => {
            if values.is_empty() {
                return;
            }
            let placeholders = vec!["?"; values.len()].join(",");
            where_parts.push(format!("{column} IN ({placeholders})"));
            for v in values {
                bound.push(Box::new(v.clone()));
            }
        }
        KeyMatch::Range(start, end) => {
            where_parts.push(format!("{column} BETWEEN ? AND ?"));
            bound.push(Box::new(start.clone()));
            bound.push(Box::new(end.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (PathBuf, Index) {
        let dir = std::env::temp_dir().join(format!("phantom-idx-{}", now_unix_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.sqlite");
        let idx = Index::open(&path).expect("open index");
        (path, idx)
    }

    #[test]
    fn open_creates_schema_and_pragmas() {
        let (_path, idx) = temp_db();
        let count = idx.total_instance_count().expect("count");
        assert_eq!(count, 0);
    }

    #[test]
    fn rescan_of_empty_dir_is_safe() {
        let (_path, idx) = temp_db();
        let dir = std::env::temp_dir().join(format!("phantom-empty-{}", now_unix_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = idx.rescan_dir(&dir).expect("scan");
        assert_eq!(report.files_seen, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_dicom_files_are_skipped_not_errored() {
        let (_path, idx) = temp_db();
        let dir = std::env::temp_dir().join(format!("phantom-junk-{}", now_unix_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), b"not dicom").unwrap();
        std::fs::write(dir.join("photo.jpg"), b"\xff\xd8\xff\xe0...not dicom").unwrap();
        let report = idx.rescan_dir(&dir).expect("scan");
        assert_eq!(report.files_seen, 2);
        assert_eq!(report.files_skipped, 2);
        assert_eq!(report.files_errored, 0);
        assert_eq!(report.files_inserted, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
