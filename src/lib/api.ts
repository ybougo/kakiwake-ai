import { Channel, invoke } from "@tauri-apps/api/core";

export type Format = "plain" | "md" | "adoc" | "textile";
export type Provider = "claude" | "codex" | "copilot";
export type Grade = "ok" | "warn" | "ng";

export const FORMATS: { id: Format; label: string; short: string }[] = [
  { id: "plain", label: "プレーンテキスト", short: "Text" },
  { id: "md", label: "Markdown", short: "Markdown" },
  { id: "adoc", label: "AsciiDoc", short: "AsciiDoc" },
  { id: "textile", label: "Redmine (Textile)", short: "Textile" },
];

export interface ProviderConfig {
  path: string;
  model: string;
  effort: string;
  extra_args: string;
}

export interface Settings {
  provider: Provider;
  providers: Record<Provider, ProviderConfig>;
  custom_models: Partial<Record<Provider, string[]>>;
  timeout_sec: number;
  quality_check: boolean;
  retry_on_ng: boolean;
  extra_instructions: Partial<Record<Format, string>>;
  last_from: Format | null;
  last_to: Format;
}

export interface ModelPreset {
  id: string;
  label: string;
  note: string;
}

export interface ProviderInfo {
  id: Provider;
  label: string;
  default_model: string;
  presets: ModelPreset[];
  efforts: string[];
}

export type AuthState =
  | { state: "logged_in"; detail: string }
  | { state: "logged_out"; detail: string }
  | { state: "unknown"; detail: string };

export interface ProviderStatus {
  id: Provider;
  ok: boolean;
  path: string | null;
  version: string | null;
  error: string | null;
  auth: AuthState | null;
}

export interface QualityReport {
  grade: Grade;
  score: number;
  recall: number;
  precision: number;
  missing_lines: string[];
  added_lines: string[];
  missing_urls: string[];
  missing_numbers: string[];
  structure: { kind: string; label: string; source: number; output: number }[];
  syntax_issues: { line: number; text: string; message: string }[];
  messages: string[];
}

export interface ConvertResponse {
  from: Format;
  to: Format;
  output: string;
  raw_output: string;
  elapsed_ms: number;
  command: string;
  quality: QualityReport | null;
  detected_from: boolean;
  provider: Provider;
  model: string;
  retried: boolean;
}

export interface ConvertRequest {
  job_id: number;
  source: string;
  from: Format | null;
  to: Format;
  provider: Provider;
  model: string;
  effort: string | null;
}

type StreamMsg = { kind: "text"; text: string } | { kind: "reset" };

export const api = {
  catalog: () => invoke<ProviderInfo[]>("provider_catalog"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  checkProviders: () => invoke<ProviderStatus[]>("check_providers"),
  openLogin: (provider: Provider) => invoke<void>("open_login", { provider }),
  /** 変換する。生成途中の文字は onText に順に届く（Codex は完了時にまとめて返る）。 */
  convert: (req: ConvertRequest, onText: (text: string) => void, onReset: () => void) => {
    const onStream = new Channel<StreamMsg>();
    onStream.onmessage = (m) => (m.kind === "text" ? onText(m.text) : onReset());
    return invoke<ConvertResponse>("convert", { req, onStream });
  },
  cancel: (jobId: number) => invoke<boolean>("cancel", { jobId }),
  /** 画面側のエラーをローカルのログファイルに残す（失敗しても無視）。 */
  logError: (context: string, message: string) =>
    invoke<void>("log_client_error", { context, message }).catch(() => {}),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
