//! Modality Worklist (DMWL) entries.
//!
//! A `WorklistEntry` is one scheduled procedure step (SPS): an
//! appointment-like row that a modality can pull via DICOM C-FIND on
//! the Modality Worklist Information Model SOP class
//! (`1.2.840.10008.5.1.4.31`). At M11 the data lives in SQLite and is
//! managed via the UI; M12 then exposes it over DIMSE.
//!
//! Schema columns mirror the DICOM key attributes a real worklist
//! response includes (PS3.4 Annex K.6.1.2.2). Optional columns are
//! nullable in SQL; required ones are NOT NULL — failing fast on a
//! malformed entry at insert time is preferable to a half-populated
//! C-FIND response later.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS worklist_entries (
    id                                         TEXT PRIMARY KEY,
    accession_number                           TEXT NOT NULL,
    patient_id                                 TEXT NOT NULL,
    patient_name                               TEXT NOT NULL,
    patient_birth_date                         TEXT,
    patient_sex                                TEXT,
    study_instance_uid                         TEXT NOT NULL,
    requested_procedure_id                     TEXT,
    requested_procedure_description            TEXT,
    scheduled_station_ae_title                 TEXT NOT NULL,
    scheduled_procedure_step_start_date        TEXT NOT NULL,
    scheduled_procedure_step_start_time        TEXT,
    scheduled_procedure_step_id                TEXT NOT NULL,
    scheduled_procedure_step_description       TEXT,
    modality                                   TEXT NOT NULL,
    created_at                                 INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wl_station     ON worklist_entries(scheduled_station_ae_title);
CREATE INDEX IF NOT EXISTS idx_wl_start_date  ON worklist_entries(scheduled_procedure_step_start_date);
CREATE INDEX IF NOT EXISTS idx_wl_patient_id  ON worklist_entries(patient_id);
CREATE INDEX IF NOT EXISTS idx_wl_accession   ON worklist_entries(accession_number);
";

/// One scheduled procedure step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorklistEntry {
    pub id: String,
    pub accession_number: String,
    pub patient_id: String,
    pub patient_name: String,
    pub patient_birth_date: Option<String>,
    pub patient_sex: Option<String>,
    pub study_instance_uid: String,
    pub requested_procedure_id: Option<String>,
    pub requested_procedure_description: Option<String>,
    pub scheduled_station_ae_title: String,
    pub scheduled_procedure_step_start_date: String,
    pub scheduled_procedure_step_start_time: Option<String>,
    pub scheduled_procedure_step_id: String,
    pub scheduled_procedure_step_description: Option<String>,
    pub modality: String,
}

/// Fields the UI provides when creating an entry. `id` is assigned by
/// the store.
#[derive(Debug, Clone, Deserialize)]
pub struct NewWorklistEntry {
    pub accession_number: String,
    pub patient_id: String,
    pub patient_name: String,
    pub patient_birth_date: Option<String>,
    pub patient_sex: Option<String>,
    pub study_instance_uid: Option<String>,
    pub requested_procedure_id: Option<String>,
    pub requested_procedure_description: Option<String>,
    pub scheduled_station_ae_title: String,
    pub scheduled_procedure_step_start_date: String,
    pub scheduled_procedure_step_start_time: Option<String>,
    pub scheduled_procedure_step_id: Option<String>,
    pub scheduled_procedure_step_description: Option<String>,
    pub modality: String,
}

/// Filters a C-FIND-RQ on the worklist SOP class — subset of the
/// DICOM matching key types that matter for a dev tool. Patterns on
/// `patient_name` use DICOM `*` / `?` wildcards (mapped to SQL `%`
/// / `_` with literal-metachar escaping, same logic as M4's store
/// query).
#[derive(Debug, Default, Clone)]
pub struct WorklistQuery {
    pub patient_id: Option<KeyMatch>,
    pub patient_name: Option<KeyMatch>,
    pub accession_number: Option<KeyMatch>,
    pub modality: Option<KeyMatch>,
    pub scheduled_station_ae_title: Option<KeyMatch>,
    pub scheduled_start_date: Option<KeyMatch>,
}

#[derive(Debug, Clone)]
pub enum KeyMatch {
    Single(String),
    Wildcard(String),
    Range(String, String),
}

/// SQLite-backed worklist store.
pub struct WorklistStore {
    conn: Mutex<Connection>,
}

impl WorklistStore {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list(&self) -> Result<Vec<WorklistEntry>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, accession_number, patient_id, patient_name,
                    patient_birth_date, patient_sex, study_instance_uid,
                    requested_procedure_id, requested_procedure_description,
                    scheduled_station_ae_title, scheduled_procedure_step_start_date,
                    scheduled_procedure_step_start_time, scheduled_procedure_step_id,
                    scheduled_procedure_step_description, modality
             FROM worklist_entries
             ORDER BY scheduled_procedure_step_start_date,
                      scheduled_procedure_step_start_time,
                      patient_name",
        )?;
        let rows = stmt
            .query_map([], map_entry_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Inserts a new entry. Required fields must be non-empty; UID
    /// fields are auto-generated when the UI does not supply them
    /// (most users do not type 64-character UIDs by hand).
    pub fn create(&self, new: NewWorklistEntry) -> Result<WorklistEntry, AppError> {
        validate(
            &new.accession_number,
            &new.patient_id,
            &new.patient_name,
            &new.scheduled_station_ae_title,
            &new.scheduled_procedure_step_start_date,
            &new.modality,
        )?;
        let entry = WorklistEntry {
            id: Uuid::new_v4().to_string(),
            accession_number: new.accession_number,
            patient_id: new.patient_id,
            patient_name: new.patient_name,
            patient_birth_date: optional(new.patient_birth_date),
            patient_sex: optional(new.patient_sex),
            study_instance_uid: new
                .study_instance_uid
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(generate_uid),
            requested_procedure_id: optional(new.requested_procedure_id),
            requested_procedure_description: optional(new.requested_procedure_description),
            scheduled_station_ae_title: new.scheduled_station_ae_title,
            scheduled_procedure_step_start_date: new.scheduled_procedure_step_start_date,
            scheduled_procedure_step_start_time: optional(new.scheduled_procedure_step_start_time),
            scheduled_procedure_step_id: new
                .scheduled_procedure_step_id
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("SPS-{}", Uuid::new_v4().simple())),
            scheduled_procedure_step_description: optional(new.scheduled_procedure_step_description),
            modality: new.modality,
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO worklist_entries (
                id, accession_number, patient_id, patient_name,
                patient_birth_date, patient_sex, study_instance_uid,
                requested_procedure_id, requested_procedure_description,
                scheduled_station_ae_title, scheduled_procedure_step_start_date,
                scheduled_procedure_step_start_time, scheduled_procedure_step_id,
                scheduled_procedure_step_description, modality, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                entry.id,
                entry.accession_number,
                entry.patient_id,
                entry.patient_name,
                entry.patient_birth_date,
                entry.patient_sex,
                entry.study_instance_uid,
                entry.requested_procedure_id,
                entry.requested_procedure_description,
                entry.scheduled_station_ae_title,
                entry.scheduled_procedure_step_start_date,
                entry.scheduled_procedure_step_start_time,
                entry.scheduled_procedure_step_id,
                entry.scheduled_procedure_step_description,
                entry.modality,
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(entry)
    }

    pub fn update(&self, entry: WorklistEntry) -> Result<WorklistEntry, AppError> {
        validate(
            &entry.accession_number,
            &entry.patient_id,
            &entry.patient_name,
            &entry.scheduled_station_ae_title,
            &entry.scheduled_procedure_step_start_date,
            &entry.modality,
        )?;
        let conn = self.lock()?;
        let rows = conn.execute(
            "UPDATE worklist_entries SET
                accession_number = ?2,
                patient_id = ?3,
                patient_name = ?4,
                patient_birth_date = ?5,
                patient_sex = ?6,
                study_instance_uid = ?7,
                requested_procedure_id = ?8,
                requested_procedure_description = ?9,
                scheduled_station_ae_title = ?10,
                scheduled_procedure_step_start_date = ?11,
                scheduled_procedure_step_start_time = ?12,
                scheduled_procedure_step_id = ?13,
                scheduled_procedure_step_description = ?14,
                modality = ?15
             WHERE id = ?1",
            params![
                entry.id,
                entry.accession_number,
                entry.patient_id,
                entry.patient_name,
                entry.patient_birth_date,
                entry.patient_sex,
                entry.study_instance_uid,
                entry.requested_procedure_id,
                entry.requested_procedure_description,
                entry.scheduled_station_ae_title,
                entry.scheduled_procedure_step_start_date,
                entry.scheduled_procedure_step_start_time,
                entry.scheduled_procedure_step_id,
                entry.scheduled_procedure_step_description,
                entry.modality,
            ],
        )?;
        if rows == 0 {
            return Err(AppError::validation(
                "id",
                format!("no worklist entry with id {}", entry.id),
            ));
        }
        Ok(entry)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let conn = self.lock()?;
        let rows = conn.execute("DELETE FROM worklist_entries WHERE id = ?1", params![id])?;
        if rows == 0 {
            return Err(AppError::validation(
                "id",
                format!("no worklist entry with id {id}"),
            ));
        }
        Ok(())
    }

    pub fn count(&self) -> Result<i64, AppError> {
        let conn = self.lock()?;
        let n: i64 =
            conn.query_row("SELECT COUNT(*) FROM worklist_entries", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Runs a Modality Worklist C-FIND query against the table.
    /// Returns the matched entries — the DMWL SCP wraps each into a
    /// response identifier with the Scheduled Procedure Step Sequence
    /// structure DICOM requires.
    pub fn find(&self, q: &WorklistQuery) -> Result<Vec<WorklistEntry>, AppError> {
        let mut where_parts: Vec<String> = Vec::new();
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        apply_match("patient_id", q.patient_id.as_ref(), &mut where_parts, &mut bound);
        apply_match("patient_name", q.patient_name.as_ref(), &mut where_parts, &mut bound);
        apply_match("accession_number", q.accession_number.as_ref(), &mut where_parts, &mut bound);
        apply_match("modality", q.modality.as_ref(), &mut where_parts, &mut bound);
        apply_match(
            "scheduled_station_ae_title",
            q.scheduled_station_ae_title.as_ref(),
            &mut where_parts,
            &mut bound,
        );
        apply_match(
            "scheduled_procedure_step_start_date",
            q.scheduled_start_date.as_ref(),
            &mut where_parts,
            &mut bound,
        );

        let mut sql = String::from(
            "SELECT id, accession_number, patient_id, patient_name,
                    patient_birth_date, patient_sex, study_instance_uid,
                    requested_procedure_id, requested_procedure_description,
                    scheduled_station_ae_title, scheduled_procedure_step_start_date,
                    scheduled_procedure_step_start_time, scheduled_procedure_step_id,
                    scheduled_procedure_step_description, modality
             FROM worklist_entries",
        );
        if !where_parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_parts.join(" AND "));
        }
        sql.push_str(
            " ORDER BY scheduled_procedure_step_start_date,
                      scheduled_procedure_step_start_time,
                      patient_name",
        );

        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|b| &**b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), map_entry_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
        self.conn
            .lock()
            .map_err(|_| AppError::Internal("worklist store mutex poisoned".to_string()))
    }
}

fn optional(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn generate_uid() -> String {
    // 2.25 is the registered OID branch for UUIDs as DICOM UIDs
    // (PS3.5 §B.2). Decimal-encoded 128-bit UUID.
    let u = Uuid::new_v4().as_u128();
    format!("2.25.{u}")
}

fn validate(
    accession_number: &str,
    patient_id: &str,
    patient_name: &str,
    scheduled_station_ae_title: &str,
    scheduled_start_date: &str,
    modality: &str,
) -> Result<(), AppError> {
    if accession_number.trim().is_empty() {
        return Err(AppError::validation(
            "accession_number",
            "Accession Number is required.",
        ));
    }
    if patient_id.trim().is_empty() {
        return Err(AppError::validation("patient_id", "Patient ID is required."));
    }
    if patient_name.trim().is_empty() {
        return Err(AppError::validation(
            "patient_name",
            "Patient Name is required.",
        ));
    }
    if scheduled_station_ae_title.trim().is_empty() {
        return Err(AppError::validation(
            "scheduled_station_ae_title",
            "Scheduled Station AE Title is required.",
        ));
    }
    if !is_valid_dicom_date(scheduled_start_date) {
        return Err(AppError::validation(
            "scheduled_procedure_step_start_date",
            "Scheduled Start Date must be eight digits, YYYYMMDD.",
        ));
    }
    if modality.trim().is_empty() {
        return Err(AppError::validation("modality", "Modality is required."));
    }
    Ok(())
}

fn is_valid_dicom_date(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_digit())
}

fn map_entry_row(row: &Row<'_>) -> rusqlite::Result<WorklistEntry> {
    Ok(WorklistEntry {
        id: row.get(0)?,
        accession_number: row.get(1)?,
        patient_id: row.get(2)?,
        patient_name: row.get(3)?,
        patient_birth_date: row.get(4)?,
        patient_sex: row.get(5)?,
        study_instance_uid: row.get(6)?,
        requested_procedure_id: row.get(7)?,
        requested_procedure_description: row.get(8)?,
        scheduled_station_ae_title: row.get(9)?,
        scheduled_procedure_step_start_date: row.get(10)?,
        scheduled_procedure_step_start_time: row.get(11)?,
        scheduled_procedure_step_id: row.get(12)?,
        scheduled_procedure_step_description: row.get(13)?,
        modality: row.get(14)?,
    })
}

/// Shared WHERE-clause builder, same shape as `store::apply_match` but
/// without the `List` variant (worklist queries do not use
/// backslash-separated UID lists in practice).
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
            let escaped = pattern
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like = escaped.replace('*', "%").replace('?', "_");
            where_parts.push(format!("{column} LIKE ? ESCAPE '\\'"));
            bound.push(Box::new(like));
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

    fn temp_store() -> (std::path::PathBuf, WorklistStore) {
        let dir = std::env::temp_dir().join(format!(
            "phantom-wl-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wl.sqlite");
        let store = WorklistStore::open(&path).expect("open");
        (path, store)
    }

    fn sample(accession: &str, station: &str) -> NewWorklistEntry {
        NewWorklistEntry {
            accession_number: accession.to_string(),
            patient_id: "P0001".to_string(),
            patient_name: "Doe^John".to_string(),
            patient_birth_date: Some("19800101".to_string()),
            patient_sex: Some("M".to_string()),
            study_instance_uid: None,
            requested_procedure_id: Some("RP-1".to_string()),
            requested_procedure_description: Some("MRI Knee".to_string()),
            scheduled_station_ae_title: station.to_string(),
            scheduled_procedure_step_start_date: "20260601".to_string(),
            scheduled_procedure_step_start_time: Some("093000".to_string()),
            scheduled_procedure_step_id: None,
            scheduled_procedure_step_description: Some("Sequence 1".to_string()),
            modality: "MR".to_string(),
        }
    }

    #[test]
    fn create_assigns_id_and_generates_uids() {
        let (_path, store) = temp_store();
        let entry = store.create(sample("A123", "MR1")).unwrap();
        assert!(!entry.id.is_empty());
        assert!(entry.study_instance_uid.starts_with("2.25."));
        assert!(entry.scheduled_procedure_step_id.starts_with("SPS-"));
    }

    #[test]
    fn missing_required_field_rejected() {
        let (_path, store) = temp_store();
        let mut bad = sample("A1", "MR1");
        bad.patient_id = String::new();
        assert!(matches!(
            store.create(bad).expect_err("should fail"),
            AppError::Validation(_)
        ));
    }

    #[test]
    fn invalid_date_rejected() {
        let (_path, store) = temp_store();
        let mut bad = sample("A1", "MR1");
        bad.scheduled_procedure_step_start_date = "2026-06-01".to_string();
        assert!(matches!(
            store.create(bad).expect_err("should fail"),
            AppError::Validation(_)
        ));
    }

    #[test]
    fn update_then_list_returns_changes() {
        let (_path, store) = temp_store();
        let mut entry = store.create(sample("A1", "MR1")).unwrap();
        entry.patient_name = "Smith^Jane".to_string();
        store.update(entry.clone()).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed[0].patient_name, "Smith^Jane");
    }

    #[test]
    fn delete_removes_entry() {
        let (_path, store) = temp_store();
        let entry = store.create(sample("A1", "MR1")).unwrap();
        store.delete(&entry.id).unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn find_by_station_and_date_range() {
        let (_path, store) = temp_store();
        store.create(sample("A1", "MR1")).unwrap();
        let mut a2 = sample("A2", "MR2");
        a2.scheduled_procedure_step_start_date = "20260605".to_string();
        store.create(a2).unwrap();

        let q = WorklistQuery {
            scheduled_station_ae_title: Some(KeyMatch::Single("MR1".to_string())),
            scheduled_start_date: Some(KeyMatch::Range(
                "20260601".to_string(),
                "20260610".to_string(),
            )),
            ..Default::default()
        };
        let rows = store.find(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].accession_number, "A1");
    }

    #[test]
    fn find_by_patient_name_wildcard() {
        let (_path, store) = temp_store();
        let mut e = sample("A1", "MR1");
        e.patient_name = "Smith^John".to_string();
        store.create(e).unwrap();
        let mut e2 = sample("A2", "MR1");
        e2.patient_name = "Jones^Alice".to_string();
        store.create(e2).unwrap();

        let q = WorklistQuery {
            patient_name: Some(KeyMatch::Wildcard("Smith*".to_string())),
            ..Default::default()
        };
        let rows = store.find(&q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].patient_name, "Smith^John");
    }
}
