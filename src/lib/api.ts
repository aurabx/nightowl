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
  | { kind: "Internal"; message: string };

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
