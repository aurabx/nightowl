// Typed bindings to the Tauri backend.
//
// Field names match the Rust `AppConfig` struct exactly (snake_case on
// both sides) so the JSON wire format and the on-disk `config.json` use
// one shared vocabulary.

import { invoke } from "@tauri-apps/api/core";

export interface AppConfig {
  local_ae_title: string;
  listen_port: number;
  store_dir: string;
}

// Shape of the error object the backend rejects with. See
// `src-tauri/src/core/error.rs` — discriminated union on `kind`.
export type AppError =
  | { kind: "Io"; message: string }
  | { kind: "Json"; message: string }
  | { kind: "Validation"; message: { field: string; reason: string } }
  | { kind: "Tauri"; message: string }
  | { kind: "Database"; message: string }
  | { kind: "DicomParse"; message: string }
  | { kind: "Internal"; message: string };

// --- Local store (M2) -------------------------------------------------

export interface ScanReport {
  files_seen: number;
  files_inserted: number;
  files_updated: number;
  files_skipped: number;
  files_errored: number;
  elapsed_ms: number;
}

export interface StudyRow {
  study_instance_uid: string;
  patient_id: string;
  patient_name: string | null;
  study_description: string | null;
  study_date: string | null;
  modalities: string | null;
  series_count: number;
  instance_count: number;
}

export interface SeriesRow {
  series_instance_uid: string;
  study_instance_uid: string;
  series_description: string | null;
  modality: string | null;
  instance_count: number;
}

export interface InstanceRow {
  sop_instance_uid: string;
  series_instance_uid: string;
  sop_class_uid: string;
  transfer_syntax_uid: string;
  file_path: string;
  size_bytes: number;
}

export function isAppError(err: unknown): err is AppError {
  return (
    typeof err === "object" &&
    err !== null &&
    typeof (err as { kind?: unknown }).kind === "string"
  );
}

/** Human-readable summary used for general error display. */
export function formatError(err: unknown): string {
  if (isAppError(err)) {
    if (err.kind === "Validation") {
      return `${err.message.field}: ${err.message.reason}`;
    }
    return `${err.kind}: ${err.message}`;
  }
  return String(err);
}

export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

export function saveConfig(cfg: AppConfig): Promise<AppConfig> {
  return invoke<AppConfig>("save_config", { cfg });
}

export function ping(): Promise<string> {
  return invoke<string>("ping");
}

// --- Local store (M2) -------------------------------------------------

export function rescanStore(): Promise<ScanReport> {
  return invoke<ScanReport>("rescan_store");
}

export function listStudies(): Promise<StudyRow[]> {
  return invoke<StudyRow[]>("list_studies");
}

export function listSeriesForStudy(studyUid: string): Promise<SeriesRow[]> {
  return invoke<SeriesRow[]>("list_series_for_study", { studyUid });
}

export function listInstancesForSeries(seriesUid: string): Promise<InstanceRow[]> {
  return invoke<InstanceRow[]>("list_instances_for_series", { seriesUid });
}

export function totalInstanceCount(): Promise<number> {
  return invoke<number>("total_instance_count");
}
