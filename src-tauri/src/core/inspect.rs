//! Single-file DICOM property reader.
//!
//! Unlike `store`, which parses a file only to extract the handful of
//! tags it indexes, this module dumps the full top-level element set of
//! a DICOM Part-10 file so the operator (or an MCP client) can inspect
//! exactly what a file contains. It is the backend for the UI's
//! drag-and-drop inspector, the `read_dicom_file` MCP tool, and the
//! `nightowl-cli inspect` subcommand.
//!
//! Scope decisions:
//!
//! - **Top-level elements only.** Sequence (`SQ`) elements are reported
//!   with their item count rather than recursed into. A full recursive
//!   dump is a larger feature; the flat view answers "what is in this
//!   file" for the common case without unbounded output.
//! - **Values are summarised, not dumped raw.** Binary VRs (pixel data,
//!   other-byte/word/float) report their byte length; long text values
//!   are truncated. The goal is a human- and LLM-readable header view,
//!   not a byte-exact export.
//! - **Any path the caller names is read.** The path comes from an OS
//!   drag-drop, an MCP argument, or a CLI argument — the caller is
//!   explicitly choosing the file, so `is_valid_name` (which guards
//!   names constructed inside the managed store directory) does not
//!   apply here.

use std::path::Path;

use dicom_core::dictionary::{DataDictionary, DataDictionaryEntry};
use dicom_core::{Tag, VR};
use dicom_dictionary_std::StandardDataDictionary;
use dicom_object::mem::InMemElement;
use dicom_object::open_file;
use serde::Serialize;

use super::error::AppError;

/// Longest rendered string value before truncation. Chosen to keep a
/// full element table readable while still showing most realistic
/// identifier, description and UID values in full.
const MAX_VALUE_CHARS: usize = 256;

/// One top-level DICOM data element, flattened for display.
#[derive(Debug, Clone, Serialize)]
pub struct DicomElement {
    /// Canonical `(gggg,eeee)` rendering, uppercase hex.
    pub tag: String,
    /// Group number, for clients that want to sort or group numerically.
    pub group: u16,
    /// Element number within the group.
    pub element: u16,
    /// Dictionary keyword (e.g. `PatientName`), or a synthesised label
    /// for tags the standard dictionary does not name.
    pub name: String,
    /// Value Representation as the two-letter code (e.g. `PN`, `UI`).
    pub vr: String,
    /// Declared value length in bytes. `None` for elements carrying an
    /// undefined length (encapsulated pixel data, undelimited
    /// sequences).
    pub length: Option<u32>,
    /// Display rendering of the value: text for string VRs, an item
    /// count for sequences, a byte-length summary for binary VRs.
    pub value: String,
}

/// Everything the inspector reports for a single dropped file.
#[derive(Debug, Clone, Serialize)]
pub struct DicomFileProperties {
    /// Absolute path the caller supplied.
    pub file_path: String,
    /// Final path component, for display.
    pub file_name: String,
    /// On-disk size in bytes.
    pub size_bytes: u64,
    /// Transfer Syntax UID from the file meta information.
    pub transfer_syntax_uid: String,
    /// Media Storage SOP Class UID from the file meta information.
    pub media_storage_sop_class_uid: String,
    /// Media Storage SOP Instance UID from the file meta information.
    pub media_storage_sop_instance_uid: String,
    /// Number of top-level data elements in the main data set.
    pub element_count: usize,
    /// The data elements, in ascending tag order.
    pub elements: Vec<DicomElement>,
}

/// Reads a DICOM Part-10 file and returns its file-meta fields plus a
/// flattened list of every top-level data element.
///
/// Returns [`AppError::DicomParse`] when the file is not a readable
/// DICOM object, and [`AppError::Io`] when the path cannot be stat-ed.
pub fn read_dicom_properties(path: &Path) -> Result<DicomFileProperties, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::Io(format!("cannot read {}: {e}", path.display())))?;

    let obj = open_file(path)
        .map_err(|e| AppError::DicomParse(format!("not a parseable DICOM file: {e}")))?;

    let meta = obj.meta();

    // `InMemDicomObject` iterates in ascending tag order (it is backed by
    // an ordered map), which is the natural reading order for a header.
    let elements: Vec<DicomElement> = obj.iter().map(element_to_view).collect();

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(DicomFileProperties {
        file_path: path.display().to_string(),
        file_name,
        size_bytes: metadata.len(),
        transfer_syntax_uid: clean_uid(&meta.transfer_syntax),
        media_storage_sop_class_uid: clean_uid(&meta.media_storage_sop_class_uid),
        media_storage_sop_instance_uid: clean_uid(&meta.media_storage_sop_instance_uid),
        element_count: elements.len(),
        elements,
    })
}

/// Projects one in-memory element to its display view.
fn element_to_view(elem: &InMemElement<StandardDataDictionary>) -> DicomElement {
    let header = elem.header();
    let tag = header.tag;

    DicomElement {
        tag: format!("({:04X},{:04X})", tag.group(), tag.element()),
        group: tag.group(),
        element: tag.element(),
        name: tag_name(tag),
        vr: header.vr.to_string().to_owned(),
        length: header.len.get(),
        value: render_value(elem),
    }
}

/// Strips the trailing NULL byte DICOM uses to pad UIDs (and any stray
/// whitespace) so meta fields display cleanly. The file meta table
/// exposes these values with their on-the-wire padding intact.
fn clean_uid(raw: &str) -> String {
    raw.trim_end_matches('\0').trim().to_string()
}

/// Resolves a tag to its dictionary keyword, falling back to a
/// synthesised label so unknown and private tags still read sensibly.
fn tag_name(tag: Tag) -> String {
    if let Some(entry) = StandardDataDictionary.by_tag(tag) {
        return entry.alias().to_string();
    }
    if tag.group() % 2 == 1 {
        "Private Tag".to_string()
    } else {
        "Unknown".to_string()
    }
}

/// Renders an element's value for display.
///
/// Sequences report an item count, binary VRs report a byte length, and
/// everything else is rendered as (truncated) text. We never attempt to
/// stringify raw binary, which would produce unreadable or misleading
/// output.
fn render_value(elem: &InMemElement<StandardDataDictionary>) -> String {
    let header = elem.header();
    match header.vr {
        VR::SQ => {
            let items = elem.items().map(|i| i.len()).unwrap_or(0);
            format!("<sequence: {items} item(s)>")
        }
        VR::OB | VR::OW | VR::OF | VR::OD | VR::OL | VR::OV | VR::UN => match header.len.get() {
            Some(bytes) => format!("<binary: {bytes} bytes>"),
            None => "<binary: undefined length>".to_string(),
        },
        _ => match elem.value().to_str() {
            Ok(text) => truncate(text.trim()),
            Err(_) => "<unrenderable value>".to_string(),
        },
    }
}

/// Truncates an over-long value to `MAX_VALUE_CHARS` characters, noting
/// the original length so the reader knows the display was clipped.
fn truncate(text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_VALUE_CHARS {
        return text.to_string();
    }
    let prefix: String = text.chars().take(MAX_VALUE_CHARS).collect();
    format!("{prefix}… ({total} chars total)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;
    use dicom_object::InMemDicomObject;
    use std::path::PathBuf;

    /// Builds a minimal but valid Part-10 file on disk and returns its
    /// path. The caller is responsible for cleanup.
    fn write_sample_file(dir: &Path) -> PathBuf {
        let mut obj = InMemDicomObject::new_empty();
        obj.put(DataElement::new(
            tags::PATIENT_NAME,
            VR::PN,
            PrimitiveValue::from("Doe^Jane"),
        ));
        obj.put(DataElement::new(
            tags::PATIENT_ID,
            VR::LO,
            PrimitiveValue::from("ABC123"),
        ));
        obj.put(DataElement::new(
            tags::MODALITY,
            VR::CS,
            PrimitiveValue::from("CT"),
        ));
        obj.put(DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.5.1.4.1.1.2"),
        ));
        obj.put(DataElement::new(
            tags::SOP_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from("1.2.3.4.5"),
        ));

        // `with_meta` copies the SOP Class / Instance UIDs from the
        // object into the file meta table; we only need to name the
        // transfer syntax explicitly.
        let file_obj = obj
            .with_meta(
                dicom_object::FileMetaTableBuilder::new().transfer_syntax("1.2.840.10008.1.2.1"),
            )
            .expect("attach meta");

        let path = dir.join("sample.dcm");
        file_obj.write_to_file(&path).expect("write sample dcm");
        path
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nightowl-inspect-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_meta_and_elements_from_a_valid_file() {
        let dir = temp_dir("valid");
        let path = write_sample_file(&dir);

        let props = read_dicom_properties(&path).expect("read properties");

        assert_eq!(props.file_name, "sample.dcm");
        assert_eq!(props.transfer_syntax_uid, "1.2.840.10008.1.2.1");
        assert_eq!(
            props.media_storage_sop_instance_uid, "1.2.3.4.5",
            "meta SOP Instance UID should round-trip"
        );
        assert_eq!(props.element_count, props.elements.len());

        let patient_name = props
            .elements
            .iter()
            .find(|e| e.name == "PatientName")
            .expect("PatientName element present");
        assert_eq!(patient_name.tag, "(0010,0010)");
        assert_eq!(patient_name.vr, "PN");
        assert_eq!(patient_name.value, "Doe^Jane");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn elements_are_in_ascending_tag_order() {
        let dir = temp_dir("order");
        let path = write_sample_file(&dir);

        let props = read_dicom_properties(&path).expect("read properties");
        let mut previous = (0u16, 0u16);
        for el in &props.elements {
            let current = (el.group, el.element);
            assert!(
                current >= previous,
                "elements out of order: {:?} after {:?}",
                current,
                previous
            );
            previous = current;
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_dicom_file_is_a_parse_error() {
        let dir = temp_dir("junk");
        let path = dir.join("not-dicom.txt");
        std::fs::write(&path, b"this is not a DICOM file").unwrap();

        let err = read_dicom_properties(&path).expect_err("should reject non-DICOM");
        assert!(
            matches!(err, AppError::DicomParse(_)),
            "expected DicomParse, got {err:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = read_dicom_properties(Path::new("/no/such/nightowl/file.dcm"))
            .expect_err("should fail to stat");
        assert!(matches!(err, AppError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn truncate_clips_and_annotates_long_values() {
        let long = "x".repeat(MAX_VALUE_CHARS + 50);
        let out = truncate(&long);
        assert!(out.contains("chars total"));
        assert!(out.chars().count() < long.chars().count());

        let short = "short";
        assert_eq!(truncate(short), short);
    }
}
