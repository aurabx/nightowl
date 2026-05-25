// Typed bindings to the Tauri backend.
//
// Field names match the Rust `AppConfig` struct exactly (snake_case on
// both sides) so the JSON wire format and the on-disk `config.json` use
// one shared vocabulary.

import { invoke } from "@tauri-apps/api/core";

export interface McpConfig {
  enabled: boolean;
  port: number;
}

export interface AppConfig {
  local_ae_title: string;
  listen_port: number;
  store_dir: string;
  mcp: McpConfig;
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

export function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
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

// --- Activity log (M9) ------------------------------------------------

export type ActivityDirection = "inbound" | "outbound" | "info";
export type ActivityStatus = "info" | "success" | "warning" | "error";

/** Matches PersistedActivityEvent + ActivityEvent flattened on the wire. */
export interface ActivityEvent {
  id: number;
  timestamp_ms: number;
  direction: ActivityDirection;
  peer_ae_title: string | null;
  peer_host: string | null;
  command: string | null;
  status: ActivityStatus;
  message: string;
  association_id: string;
}

export interface ActivityFilter {
  direction?: ActivityDirection;
  status?: ActivityStatus;
  peer_ae_title?: string;
  command?: string;
  association_id?: string;
  search?: string;
  since_ms?: number;
  limit?: number;
  offset?: number;
}

export interface ActivityPage {
  events: ActivityEvent[];
  total: number;
}

export function listActivity(filter?: ActivityFilter): Promise<ActivityPage> {
  return invoke<ActivityPage>("list_activity", { filter });
}

export function clearActivity(): Promise<void> {
  return invoke<void>("clear_activity");
}

export function activityCount(): Promise<number> {
  return invoke<number>("activity_count");
}

// --- Peers (M7) ------------------------------------------------------

export interface Peer {
  id: string;
  name: string;
  ae_title: string;
  host: string;
  port: number;
}

export interface NewPeer {
  name: string;
  ae_title: string;
  host: string;
  port: number;
}

export interface UpdatePeer {
  id: string;
  name: string;
  ae_title: string;
  host: string;
  port: number;
}

export function listPeers(): Promise<Peer[]> {
  return invoke<Peer[]>("list_peers");
}

export function createPeer(peer: NewPeer): Promise<Peer> {
  return invoke<Peer>("create_peer", { peer });
}

export function updatePeer(peer: UpdatePeer): Promise<Peer> {
  return invoke<Peer>("update_peer", { peer });
}

export function deletePeer(id: string): Promise<void> {
  return invoke<void>("delete_peer", { id });
}

// --- SCU operations (M8) ---------------------------------------------

export type QrRoot = "patient" | "study";
export type QrLevel = "PATIENT" | "STUDY" | "SERIES" | "IMAGE";

export interface ScuQueryKeys {
  patient_id?: string;
  patient_name?: string;
  study_instance_uid?: string;
  study_date?: string;
  modality?: string;
  series_instance_uid?: string;
  sop_instance_uid?: string;
  /** Empty-valued tag names to also include as return keys. */
  return_keys?: string[];
}

export interface ScuEchoResult {
  success: boolean;
  status: number;
  elapsed_ms: number;
  message: string;
}

export interface ScuFindMatch {
  fields: Record<string, string>;
}

export interface ScuFindResult {
  matches: ScuFindMatch[];
  elapsed_ms: number;
}

export interface ScuMoveResult {
  completed: number;
  failed: number;
  status: number;
  status_label: string;
  elapsed_ms: number;
}

export interface ScuStoreOutcome {
  file: string;
  success: boolean;
  sop_instance_uid: string | null;
  message: string;
}

export function scuEcho(peerId: string): Promise<ScuEchoResult> {
  return invoke<ScuEchoResult>("scu_echo_cmd", { peerId });
}

export function scuFind(
  peerId: string,
  root: QrRoot,
  level: QrLevel,
  keys: ScuQueryKeys,
): Promise<ScuFindResult> {
  return invoke<ScuFindResult>("scu_find_cmd", { peerId, root, level, keys });
}

export function scuMove(
  peerId: string,
  root: QrRoot,
  level: QrLevel,
  keys: ScuQueryKeys,
  destinationAe: string,
): Promise<ScuMoveResult> {
  return invoke<ScuMoveResult>("scu_move_cmd", {
    peerId,
    root,
    level,
    keys,
    destinationAe,
  });
}

export function scuStore(
  peerId: string,
  files: string[],
): Promise<ScuStoreOutcome[]> {
  return invoke<ScuStoreOutcome[]>("scu_store_cmd", { peerId, files });
}

// --- Worklist (M11) --------------------------------------------------

export interface WorklistEntry {
  id: string;
  accession_number: string;
  patient_id: string;
  patient_name: string;
  patient_birth_date: string | null;
  patient_sex: string | null;
  study_instance_uid: string;
  requested_procedure_id: string | null;
  requested_procedure_description: string | null;
  scheduled_station_ae_title: string;
  scheduled_procedure_step_start_date: string;
  scheduled_procedure_step_start_time: string | null;
  scheduled_procedure_step_id: string;
  scheduled_procedure_step_description: string | null;
  modality: string;
}

export interface NewWorklistEntry {
  accession_number: string;
  patient_id: string;
  patient_name: string;
  patient_birth_date?: string;
  patient_sex?: string;
  study_instance_uid?: string;
  requested_procedure_id?: string;
  requested_procedure_description?: string;
  scheduled_station_ae_title: string;
  scheduled_procedure_step_start_date: string;
  scheduled_procedure_step_start_time?: string;
  scheduled_procedure_step_id?: string;
  scheduled_procedure_step_description?: string;
  modality: string;
}

export function listWorklist(): Promise<WorklistEntry[]> {
  return invoke<WorklistEntry[]>("list_worklist");
}

export function createWorklistEntry(entry: NewWorklistEntry): Promise<WorklistEntry> {
  return invoke<WorklistEntry>("create_worklist_entry", { entry });
}

export function updateWorklistEntry(entry: WorklistEntry): Promise<WorklistEntry> {
  return invoke<WorklistEntry>("update_worklist_entry", { entry });
}

export function deleteWorklistEntry(id: string): Promise<void> {
  return invoke<void>("delete_worklist_entry", { id });
}
