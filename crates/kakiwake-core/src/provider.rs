//! LLM CLI（claude / codex / copilot）の単発実行。
//!
//! - 本文は必ず stdin で渡す（コマンドライン長の制限・エスケープ問題を避ける）
//! - ツールは無効化し、空の一時ディレクトリをカレントにして実行する
//! - 自動更新・テレメトリ・MCP など LLM 呼び出し以外の通信は可能な限り止める

use crate::prompt::Prompt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    Copilot,
}

/// GUI のモデル選択に出す候補。先頭が既定。
#[derive(Debug, Clone, Serialize)]
pub struct ModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Copilot];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Copilot => "copilot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex CLI",
            Provider::Copilot => "GitHub Copilot CLI",
        }
    }

    pub fn program(self) -> &'static str {
        self.id()
    }

    pub fn default_model(self) -> &'static str {
        self.presets()[0].id
    }

    pub fn presets(self) -> &'static [ModelPreset] {
        match self {
            Provider::Claude => &[
                ModelPreset { id: "claude-sonnet-5-5", label: "Sonnet 5.5", note: "既定・安定" },
                ModelPreset {
                    id: "claude-haiku-5-5", label: "Haiku 5.5", note: "最速・まれに余計な行を足す"
                },
                ModelPreset { id: "claude-opus-5-5", label: "Opus 5.5", note: "最高品質・低速" },
            ],
            Provider::Codex => &[
                ModelPreset { id: "gpt-6.1-terra", label: "GPT-6.1 Terra", note: "既定・軽量" },
                ModelPreset { id: "gpt-6.2-sol", label: "GPT-6.2 Sol", note: "高品質" },
                ModelPreset { id: "gpt-6.1-sol", label: "GPT-6.1 Sol", note: "" },
            ],
            Provider::Copilot => &[
                ModelPreset { id: "mai-code-2", label: "MAI Code 2", note: "既定・安定" },
                ModelPreset {
                    id: "mai-code-2-flash", label: "MAI Code 2 Flash", note: "やや速い・崩れることがある"
                },
                ModelPreset { id: "auto", label: "Auto", note: "Copilot が自動選択" },
            ],
        }
    }

    /// 推論量の既定値（変換は推論をほぼ必要としないので低め）。
    /// 推論量の既定値。
    /// - Codex: "none" でも品質は同じ（gpt-6.1-terra は "minimal" 非対応）
    /// - Copilot: "none" だと Textile の見出し・表を変換し損ねることがあるため "low"
    pub fn default_effort(self) -> &'static str {
        match self {
            Provider::Claude => "low",
            Provider::Codex => "none",
            Provider::Copilot => "low",
        }
    }

    /// GUI に出す推論量の選択肢（"" は CLI の既定）。
    pub fn efforts(self) -> &'static [&'static str] {
        match self {
            Provider::Claude => &["", "low", "medium", "high"],
            Provider::Codex => &["", "none", "low", "medium", "high"],
            Provider::Copilot => &["", "none", "low", "medium", "high"],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// 空なら PATH から探す
    pub path: String,
    pub model: String,
    /// 空なら指定しない
    pub effort: String,
    /// ユーザーが追加した任意の引数（空白区切り）
    pub extra_args: String,
}

impl ProviderConfig {
    pub fn for_provider(p: Provider) -> Self {
        Self { model: p.default_model().into(), effort: p.default_effort().into(), ..Default::default() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{0} が見つかりません。インストールするか、設定で実行ファイルのパスを指定してください。")]
    NotFound(String),
    #[error("{0} の起動に失敗しました: {1}")]
    Spawn(String, std::io::Error),
    #[error("タイムアウトしました（{0} 秒）")]
    Timeout(u64),
    #[error("キャンセルしました")]
    Cancelled,
    #[error("{provider} がエラー終了しました（code {code:?}）: {stderr}")]
    Failed { provider: String, code: Option<i32>, stderr: String },
    #[error(
        "Claude Code の安全フィルターが変換を拒否しました。変換先の「追加指示」に推論過程の出力を求める指示があれば外してください。文書変換だけでも再現する場合は、下の Request ID を Anthropic サポートに伝えてください。\n\n{detail}"
    )]
    ClaudeReasoningRefusal { detail: String },
    #[error("{provider} がツールを使おうとしたため中止しました（{kind}{}）。文書に指示文が紛れていないか確認してください。", if .target.is_empty() { String::new() } else { format!(": {}", .target) })]
    ToolUse { provider: String, kind: String, target: String },
    #[error("{provider} にログインしていません。{hint}\n{detail}")]
    Auth { provider: String, hint: String, detail: String },
    #[error("{0} の出力が空でした。ログイン状態やモデル名を確認してください。{1}")]
    Empty(String, String),
    #[error("入出力エラー: {0}")]
    Io(#[from] std::io::Error),
}

impl RunError {
    /// ログ用の要約。CLI の出力やエラー本文には文書の内容が含まれ得るので、種類と長さだけにする。
    pub fn log_summary(&self) -> String {
        let chars = |s: &str| s.chars().count();
        match self {
            RunError::NotFound(p) => format!("not_found: {p}"),
            RunError::Spawn(p, e) => format!("spawn_failed: {p}: {e}"),
            RunError::Timeout(s) => format!("timeout: {s}s"),
            RunError::Cancelled => "cancelled".into(),
            RunError::Failed { code, stderr, .. } => format!("failed: code={code:?} detail_chars={}", chars(stderr)),
            RunError::ClaudeReasoningRefusal { .. } => "refusal: reasoning_extraction".into(),
            RunError::ToolUse { kind, target, .. } => {
                // ファイル変更は書き換えられたパスが分からないと確認できないので残す
                if kind == "file_change" { format!("tool_use: {kind}: {target}") } else { format!("tool_use: {kind}") }
            }
            RunError::Auth { detail, .. } => format!("auth: detail_chars={}", chars(detail)),
            RunError::Empty(_, stderr) => format!("empty_output: stderr_chars={}", chars(stderr)),
            RunError::Io(e) => format!("io: {}", e.kind()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RunOutput {
    pub text: String,
    pub elapsed_ms: u128,
    pub command: String,
}

/// 実行ファイルを解決する（Windows では PATHEXT を考慮して .exe / .cmd も探す）。
pub fn resolve(provider: Provider, cfg: &ProviderConfig) -> Result<PathBuf, RunError> {
    let p = cfg.path.trim();
    if !p.is_empty() {
        let path = PathBuf::from(p);
        return if path.is_file() { Ok(path) } else { Err(RunError::NotFound(p.to_string())) };
    }
    if let Ok(found) = which::which(provider.program()) {
        return Ok(found);
    }
    // アプリ起動後に CLI を入れた場合など、プロセスの PATH が古いときは既定のインストール先も探す
    let dirs = fallback_dirs();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
    if !dirs.is_empty()
        && let Ok(found) =
            which::which_in(provider.program(), Some(std::env::join_paths(&dirs).unwrap_or_default()), cwd)
    {
        return Ok(found);
    }
    Err(RunError::NotFound(provider.program().to_string()))
}

/// 主なインストール先（レジストリ上の最新 PATH、winget / npm / 各公式インストーラー）。
fn fallback_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut dirs = registry_path_dirs();
    if let Some(local) = env("LOCALAPPDATA") {
        dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
        dirs.push(local.join("GitHubCopilotCLI"));
        dirs.push(local.join("Programs").join("claude"));
    }
    if let Some(appdata) = env("APPDATA") {
        dirs.push(appdata.join("npm"));
    }
    if let Some(home) = env("USERPROFILE").or_else(|| env("HOME")) {
        dirs.push(home.join(".local").join("bin"));
    }
    if let Some(pf) = env("ProgramFiles") {
        dirs.push(pf.join("GitHub Copilot CLI"));
        dirs.push(pf.join("nodejs"));
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// レジストリに保存されている現在のユーザー PATH とシステム PATH。
/// winget などで CLI を入れた後、アプリを再起動しなくても見つけられるようにする。
#[cfg(windows)]
fn registry_path_dirs() -> Vec<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut dirs = Vec::new();
    for key in ["HKCU\\Environment", "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"] {
        let Ok(out) = std::process::Command::new("reg.exe")
            .args(["query", key, "/v", "Path"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(value) = parse_reg_path(&text) {
            dirs.extend(std::env::split_paths(&expand_env(&value)));
        }
    }
    dirs
}

#[cfg(not(windows))]
fn registry_path_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// `reg query ... /v Path` の出力から値を取り出す。
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_path(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("Path").or_else(|| l.strip_prefix("PATH"))?;
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("REG_EXPAND_SZ").or_else(|| rest.strip_prefix("REG_SZ"))?;
        Some(rest.trim().to_string())
    })
}

/// `%LOCALAPPDATA%` のような環境変数を展開する。
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// CLI のバージョン文字列を取得（存在確認用）。
pub async fn version(provider: Provider, cfg: &ProviderConfig) -> Result<String, RunError> {
    let exe = resolve(provider, cfg)?;
    let mut cmd = base_command(&exe);
    if provider == Provider::Copilot {
        cmd.arg("--no-auto-update");
    }
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    apply_env(&mut cmd, provider);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| RunError::Timeout(20))?
        .map_err(|e| RunError::Spawn(provider.program().into(), e))?;
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines().next().unwrap_or("").trim().to_string())
}

/// ログイン状態。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum AuthState {
    LoggedIn(String),
    LoggedOut(String),
    /// CLI に状態確認コマンドがない（Copilot）など
    Unknown(String),
}

impl Provider {
    /// ターミナルで実行するログインコマンド（引数部分）。
    pub fn login_args(self) -> &'static [&'static str] {
        match self {
            Provider::Claude => &["auth", "login"],
            Provider::Codex => &["login"],
            Provider::Copilot => &["login"],
        }
    }

    /// 未ログイン時に表示する案内。
    pub fn login_hint(self) -> &'static str {
        match self {
            Provider::Claude => {
                "ターミナルで `claude auth login` を実行してください（API キーを使う場合は環境変数 ANTHROPIC_API_KEY）。"
            }
            Provider::Codex => {
                "ターミナルで `codex login` を実行してください（API キーを使う場合は環境変数 OPENAI_API_KEY）。"
            }
            Provider::Copilot => {
                "ターミナルで `copilot login` を実行してください（トークンを使う場合は環境変数 COPILOT_GITHUB_TOKEN / GH_TOKEN）。"
            }
        }
    }
}

/// ローカルの認証情報からログイン状態を確認する（Claude: `auth status`、Codex: `login status`）。
pub async fn auth_status(provider: Provider, cfg: &ProviderConfig) -> Result<AuthState, RunError> {
    let exe = resolve(provider, cfg)?;
    let args: &[&str] = match provider {
        Provider::Claude => &["auth", "status"],
        Provider::Codex => &["login", "status"],
        Provider::Copilot => {
            let has_token = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
                .iter()
                .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
            return Ok(AuthState::Unknown(if has_token {
                "環境変数のトークンを使用します".into()
            } else {
                "Copilot CLI には状態確認コマンドがありません。未ログインなら変換時にエラーになります。".into()
            }));
        }
    };
    let mut cmd = base_command(&exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Ok(dir) = WorkDir::new() {
        cmd.current_dir(&dir.0);
        apply_env(&mut cmd, provider);
        let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
            .await
            .map_err(|_| RunError::Timeout(20))?
            .map_err(|e| RunError::Spawn(provider.program().into(), e))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Ok(parse_auth(provider, out.status.success(), &stdout, &stderr));
    }
    Ok(AuthState::Unknown(String::new()))
}

fn parse_auth(provider: Provider, success: bool, stdout: &str, stderr: &str) -> AuthState {
    match provider {
        Provider::Claude => match serde_json::from_str::<serde_json::Value>(stdout) {
            Ok(v) if v["loggedIn"].as_bool() == Some(true) => {
                AuthState::LoggedIn(v["authMethod"].as_str().unwrap_or("ログイン済み").to_string())
            }
            Ok(_) => AuthState::LoggedOut(provider.login_hint().into()),
            Err(_) => AuthState::Unknown(tail(stdout, 200)),
        },
        _ => {
            let text = format!("{stdout}\n{stderr}");
            if success
                && text.to_ascii_lowercase().contains("logged in")
                && !text.to_ascii_lowercase().contains("not logged in")
            {
                // codex は状態を stderr に出す
                let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ログイン済み");
                AuthState::LoggedIn(line.to_string())
            } else {
                AuthState::LoggedOut(provider.login_hint().into())
            }
        }
    }
}

/// 認証エラーらしい出力か。
fn looks_like_auth_error(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "not logged in",
        "please log in",
        "please login",
        "/login",
        "unauthorized",
        "authentication",
        "401",
        "invalid api key",
        "oauth token",
        "credentials",
    ]
    .iter()
    .any(|k| t.contains(k))
}

static HELP_CACHE: LazyLock<Mutex<HashMap<PathBuf, String>>> = LazyLock::new(Default::default);

/// 実行する版の `--help`（任意フラグの対応確認用。プロセス内でキャッシュ）。
async fn help_text(provider: Provider, exe: &Path) -> String {
    if provider != Provider::Copilot {
        return String::new();
    }
    if let Some(h) = HELP_CACHE.lock().unwrap().get(exe) {
        return h.clone();
    }
    let mut cmd = base_command(exe);
    // 自動更新を止めた状態で、実際に実行される版のヘルプを取る
    cmd.args(["--no-auto-update", "--help"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    apply_env(&mut cmd, provider);
    let help = match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    };
    HELP_CACHE.lock().unwrap().insert(exe.to_path_buf(), help.clone());
    help
}

fn base_command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        // コンソールウィンドウを出さない
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// LLM 呼び出し以外の通信（自動更新・テレメトリ・エラー報告など）を止める環境変数。
fn apply_env(cmd: &mut Command, provider: Provider) {
    match provider {
        Provider::Claude => {
            cmd.env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
                .env("DISABLE_TELEMETRY", "1")
                .env("DISABLE_ERROR_REPORTING", "1")
                .env("DISABLE_AUTOUPDATER", "1")
                .env("DISABLE_BUG_COMMAND", "1");
        }
        Provider::Codex => {
            // 更新確認は -c check_for_update_on_startup=false で止める（build_args 参照）
        }
        Provider::Copilot => {
            cmd.env("COPILOT_AUTO_UPDATE", "false")
                .env("COPILOT_DISABLE_DESKTOP_NOTIFICATIONS", "1")
                .env("COPILOT_MCP_TOOL_CACHE", "false");
        }
    }
    cmd.env("NO_COLOR", "1");
}

/// Codex で無効にする機能。変換にはどれも不要。
/// - shell_tool / unified_exec: シェルの実行（read-only でも任意のファイルを読める）
/// - apply_patch_freeform: 自由形式のファイル編集
/// - view_image: ローカル画像の読み込み
/// - apps / plugins / tool_suggest: ChatGPT 連携アプリ（サイト公開・共有設定の変更・Web 検索など）
/// - standalone_web_search: Web 検索
/// - multi_agent / multi_agent_v2 / enable_fanout: サブエージェント
/// - その他: 画像生成、メモリ、目標管理、権限要求、コード実行モード
const CODEX_DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "apply_patch_freeform",
    "view_image",
    "apps",
    "plugins",
    "tool_suggest",
    "standalone_web_search",
    "multi_agent",
    "multi_agent_v2",
    "enable_fanout",
    "image_generation",
    "memories",
    "goals",
    "request_permissions_tool",
    "code_mode",
];

/// Codex の JSON イベントのうち、ツールの実行を表す item の種類。
/// これが出たら変換としては異常なので、プロセスを止めて利用者に知らせる。
const CODEX_TOOL_ITEMS: &[&str] =
    &["command_execution", "file_change", "mcp_tool_call", "web_search", "collab_tool_call", "image_generation"];

/// 実行する引数を組み立てる（テストと表示用に分離）。
pub fn build_args(provider: Provider, cfg: &ProviderConfig, last_message_file: &Path, help: &str) -> Vec<String> {
    build_args_with_preamble(provider, cfg, last_message_file, help, crate::prompt::SYSTEM_PREAMBLE)
}

fn build_args_with_preamble(
    provider: Provider,
    cfg: &ProviderConfig,
    last_message_file: &Path,
    help: &str,
    preamble: &'static str,
) -> Vec<String> {
    let model = if cfg.model.trim().is_empty() { provider.default_model() } else { cfg.model.trim() };
    let effort = cfg.effort.trim();
    let mut a: Vec<String> = Vec::new();
    match provider {
        Provider::Claude => {
            a.extend(
                ["-p", "--model", model, "--tools", "", "--strict-mcp-config", "--no-session-persistence"]
                    .map(String::from),
            );
            // 生成途中の文字を受け取るため stream-json（JSON Lines）で出力させる
            a.extend(
                ["--setting-sources", "", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]
                    .map(String::from),
            );
            // Windows の .cmd シム経由でも安全に渡せるよう、引数は改行なしの短い 1 行だけにする。
            // 変換規則と本文はすべて stdin で渡す。
            a.push("--system-prompt".into());
            a.push(preamble.into());
            if !effort.is_empty() {
                a.extend(["--effort".into(), effort.into()]);
            }
        }
        Provider::Codex => {
            a.extend(
                ["exec", "-m", model, "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral"]
                    .map(String::from),
            );
            a.extend(["--color", "never"].map(String::from));
            // MCP サーバー・Web 検索・分析送信を無効化
            a.extend(
                [
                    "-c",
                    "mcp_servers={}",
                    "-c",
                    "web_search=\"disabled\"",
                    "-c",
                    "analytics.enabled=false",
                    "-c",
                    "check_for_update_on_startup=false",
                ]
                .map(String::from),
            );
            // ツールを止める。read-only サンドボックスでもシェルは任意のファイルを読め、apply_patch は
            // 作業ディレクトリ外にも書けた（codex 0.162 で確認）。止められる機能はすべて止める。
            // 未知のキーは Codex が警告して無視するだけなので、版が変わっても起動は妨げない。
            for feature in CODEX_DISABLED_FEATURES {
                a.extend(["-c".into(), format!("features.{feature}=false")]);
            }
            if !effort.is_empty() {
                a.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
            }
            // ツール実行を検知するためにイベントを JSON Lines で受け取る（最終結果は -o のファイル）
            a.push("--json".into());
            a.push("-o".into());
            a.push(last_message_file.to_string_lossy().into_owned());
            // プロンプト引数を省略すると stdin 全体が指示として読まれる
            a.push("-".into());
        }
        Provider::Copilot => {
            // copilot は -p の文字列しかプロンプトにしないため、-p を付けず stdin 全体をプロンプトにする
            // --available-tools= （空）は「制限なし」と解釈され bash が動いた。存在しない名前を指定して
            // モデルに見えるツールを 0 にし、さらに拒否ルールで全種類を禁止する（拒否は許可より優先）。
            a.extend(
                ["-s", "--model", model, "--no-auto-update", "--no-custom-instructions", "--available-tools=none"]
                    .map(String::from),
            );
            if help.contains("--deny-tool") {
                for kind in ["shell", "write", "read", "memory", "url"] {
                    a.push(format!("--deny-tool={kind}"));
                }
            }
            // バージョンによって無いフラグは --help で確認できたものだけ付ける
            for flag in ["--disable-builtin-mcps", "--no-ask-user", "--disallow-temp-dir", "--disable-client-telemetry"]
            {
                if help.contains(flag) {
                    a.push(flag.into());
                }
            }
            if !effort.is_empty() {
                a.extend(["--reasoning-effort".into(), effort.into()]);
            }
        }
    }
    a.extend(cfg.extra_args.split_whitespace().map(String::from));
    a
}

/// stdin に流す内容。どの CLI も「変換規則＋本文」を 1 本で受け取る。
fn stdin_payload(prompt: &Prompt) -> String {
    prompt.combined()
}

fn display_command(exe: &Path, args: &[String]) -> String {
    let mut s = exe.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    for a in args {
        s.push(' ');
        if a.contains('\n') || a.len() > 60 {
            s.push_str("<…>");
        } else if a.is_empty() || a.contains(' ') {
            s.push_str(&format!("\"{a}\""));
        } else {
            s.push_str(a);
        }
    }
    s
}

/// 一時作業ディレクトリ。CLI がファイルを読んでも何もない場所にする。
struct WorkDir(PathBuf);

/// 同じプロセス内で作る一時ディレクトリの通し番号。
static WORK_DIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl WorkDir {
    fn new() -> std::io::Result<Self> {
        Self::new_in(&std::env::temp_dir())
    }

    /// 並行して作っても名前が重ならないようにする。
    /// （Windows の時計は粗く、時刻だけだと同じ名前になり、片方の削除でもう片方が消えていた）
    fn new_in(base: &Path) -> std::io::Result<Self> {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        loop {
            let seq = WORK_DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = base.join(format!("kakiwake-{}-{nanos}-{seq}", std::process::id()));
            // create_dir は既にあると失敗するので、他と共有することがない
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 子プロセスの終了後、パイプの後始末を待つ上限。
const PIPE_GRACE: Duration = Duration::from_secs(3);

/// 生成途中のテキストを受け取るコールバック。
pub type OnText<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// stdout を読みながら最終テキストを組み立てた結果。
#[derive(Default)]
struct Collected {
    /// 最終テキスト（Claude は result、Copilot は stdout 全体、Codex は -o のファイル）
    text: String,
    /// 生の stdout（エラー表示用）
    raw: String,
    /// CLI が自分で報告したエラー（Claude の is_error など）
    error: Option<String>,
    /// 危険な動作を検知したので、CLI の終了を待たずに止める
    abort: bool,
    /// 検知したツールの使用（種類, 対象）
    tool_use: Option<(String, String)>,
}

pub async fn run(
    provider: Provider,
    cfg: &ProviderConfig,
    prompt: &Prompt,
    timeout: Duration,
    cancel: Option<oneshot::Receiver<()>>,
    on_text: Option<OnText<'_>>,
) -> Result<RunOutput, RunError> {
    let exe = resolve(provider, cfg)?;
    let work = WorkDir::new()?;
    let last_msg = work.0.join("last-message.txt");
    let help = help_text(provider, &exe).await;
    let args = build_args_with_preamble(provider, cfg, &last_msg, &help, prompt.claude_preamble());
    let command = display_command(&exe, &args);

    let mut cmd = base_command(&exe);
    cmd.args(&args).current_dir(&work.0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    apply_env(&mut cmd, provider);

    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| RunError::Spawn(provider.program().into(), e))?;
    let payload = stdin_payload(prompt);
    let mut stdin = child.stdin.take().expect("stdin piped");
    let mut writer = tokio::spawn(async move {
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.shutdown().await;
    });
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let mut stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });
    let stdout_pipe = child.stdout.take().expect("stdout piped");

    enum Outcome {
        Done(std::io::Result<(Collected, std::process::ExitStatus)>),
        TimedOut,
        Cancelled,
    }
    let outcome = {
        let work_fut = async {
            let collected = collect_stdout(provider, stdout_pipe, on_text).await?;
            if collected.abort {
                let _ = child.start_kill();
            }
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((collected, status))
        };
        match cancel {
            Some(rx) => tokio::select! {
                r = tokio::time::timeout(timeout, work_fut) => r.map_or(Outcome::TimedOut, Outcome::Done),
                _ = rx => Outcome::Cancelled,
            },
            None => tokio::time::timeout(timeout, work_fut).await.map_or(Outcome::TimedOut, Outcome::Done),
        }
    };
    let (collected, status) = match outcome {
        Outcome::Done(r) => r?,
        Outcome::TimedOut | Outcome::Cancelled => {
            // 子プロセスを止めてから戻る（止めないと stdin の書き込みや stderr の読み込みが終わらない）
            let _ = child.start_kill();
            let _ = tokio::time::timeout(PIPE_GRACE, child.wait()).await;
            writer.abort();
            stderr_task.abort();
            return Err(if matches!(outcome, Outcome::Cancelled) {
                RunError::Cancelled
            } else {
                RunError::Timeout(timeout.as_secs())
            });
        }
    };
    // 子プロセスは終了済み。孫プロセスがパイプを握っていても待ち続けないよう上限を設ける
    if tokio::time::timeout(PIPE_GRACE, &mut writer).await.is_err() {
        writer.abort();
    }
    let stderr = match tokio::time::timeout(PIPE_GRACE, &mut stderr_task).await {
        Ok(r) => r.unwrap_or_default(),
        Err(_) => {
            stderr_task.abort();
            String::new()
        }
    };

    if let Some((kind, target)) = collected.tool_use {
        return Err(RunError::ToolUse { provider: provider.label().into(), kind, target });
    }
    let failed = !status.success() || collected.error.is_some();
    if failed || (collected.text.trim().is_empty() && looks_like_auth_error(&stderr)) {
        let detail = collected
            .error
            .clone()
            .or_else(|| (!stderr.trim().is_empty()).then(|| stderr.clone()))
            .unwrap_or_else(|| collected.raw.clone());
        if provider == Provider::Claude
            && detail.contains("safeguards flagged this message")
            && detail.contains("[reasoning_extraction]")
        {
            return Err(RunError::ClaudeReasoningRefusal { detail: tail(&detail, 1200) });
        }
        if looks_like_auth_error(&detail) {
            return Err(RunError::Auth {
                provider: provider.label().into(),
                hint: provider.login_hint().into(),
                detail: tail(&detail, 400),
            });
        }
        return Err(RunError::Failed {
            provider: provider.label().into(),
            code: status.code(),
            stderr: tail(&detail, 1200),
        });
    }

    let text = match provider {
        Provider::Codex => std::fs::read_to_string(&last_msg).unwrap_or(collected.text),
        _ => collected.text,
    };
    if text.trim().is_empty() {
        return Err(RunError::Empty(provider.label().into(), tail(&stderr, 600)));
    }
    Ok(RunOutput { text, elapsed_ms: started.elapsed().as_millis(), command })
}

/// stdout を読み、生成途中のテキストを `on_text` に流しながら最終テキストを集める。
async fn collect_stdout(
    provider: Provider,
    mut pipe: tokio::process::ChildStdout,
    on_text: Option<OnText<'_>>,
) -> std::io::Result<Collected> {
    let mut c = Collected::default();
    match provider {
        Provider::Claude => {
            // JSON Lines: 途中経過は content_block_delta、最終結果は result
            let mut lines = BufReader::new(pipe).lines();
            let mut streamed = String::new();
            while let Some(line) = lines.next_line().await? {
                c.raw.push_str(&line);
                c.raw.push('\n');
                match parse_claude_line(&line) {
                    Some(ClaudeEvent::Delta(t)) => {
                        if let Some(f) = on_text {
                            f(&t);
                        }
                        streamed.push_str(&t);
                    }
                    Some(ClaudeEvent::Result(t)) => c.text = t,
                    Some(ClaudeEvent::Error(e)) => c.error = Some(e),
                    None => {}
                }
            }
            if c.text.is_empty() {
                c.text = streamed;
            }
        }
        Provider::Copilot => {
            // 素のテキストが少しずつ出る。UTF-8 の途中で切れないよう、確定した部分だけ流す
            let mut buf = [0u8; 4096];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                let n = pipe.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n]);
                let valid = match std::str::from_utf8(&pending) {
                    Ok(_) => pending.len(),
                    Err(e) => e.valid_up_to(),
                };
                if valid > 0 {
                    let chunk = String::from_utf8_lossy(&pending[..valid]).into_owned();
                    pending.drain(..valid);
                    if let Some(f) = on_text {
                        f(&chunk);
                    }
                    c.text.push_str(&chunk);
                }
            }
            if !pending.is_empty() {
                c.text.push_str(&String::from_utf8_lossy(&pending));
            }
            c.raw = c.text.clone();
        }
        Provider::Codex => {
            // codex exec は途中経過のテキストを出さない。最終テキストは -o のファイルから読む。
            // イベントを監視し、ツールの実行が出たら即座に打ち切る。
            let mut lines = BufReader::new(pipe).lines();
            while let Some(line) = lines.next_line().await? {
                c.raw.push_str(&line);
                c.raw.push('\n');
                if let Some(found) = codex_tool_use(&line) {
                    c.tool_use = Some(found);
                    c.abort = true;
                    break;
                }
            }
        }
    }
    Ok(c)
}

/// Codex の JSON イベント 1 行がツールの実行なら、（種類, 対象）を返す。
fn codex_tool_use(line: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if !v["type"].as_str()?.starts_with("item.") {
        return None;
    }
    let item = &v["item"];
    let kind = item["type"].as_str()?;
    if !CODEX_TOOL_ITEMS.contains(&kind) {
        return None;
    }
    let detail = match kind {
        "command_execution" => item["command"].as_str().unwrap_or_default().to_string(),
        "file_change" => item["changes"]
            .as_array()
            .map(|cs| cs.iter().filter_map(|c| c["path"].as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default(),
        _ => item["tool"].as_str().or(item["query"].as_str()).unwrap_or_default().to_string(),
    };
    let detail: String = detail.chars().take(200).collect();
    Some((kind.to_string(), detail))
}

#[derive(Debug, PartialEq)]
enum ClaudeEvent {
    Delta(String),
    Result(String),
    Error(String),
}

fn parse_claude_line(line: &str) -> Option<ClaudeEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v["type"].as_str()? {
        "stream_event" => {
            let e = &v["event"];
            (e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta")
                .then(|| ClaudeEvent::Delta(e["delta"]["text"].as_str().unwrap_or_default().to_string()))
        }
        "result" => {
            let text = v["result"].as_str().unwrap_or_default().to_string();
            if v["is_error"].as_bool() == Some(true) || v["subtype"].as_str().is_some_and(|s| s != "success") {
                Some(ClaudeEvent::Error(if text.is_empty() { line.to_string() } else { text }))
            } else {
                Some(ClaudeEvent::Result(text))
            }
        }
        _ => None,
    }
}

fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= max { s.to_string() } else { format!("…{}", s.chars().skip(n - max).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_dirs_created_concurrently_are_distinct() {
        let base = std::env::temp_dir().join(format!("kakiwake-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let mut dirs: Vec<WorkDir> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..16).map(|_| s.spawn(|| WorkDir::new_in(&base).unwrap())).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut paths: Vec<_> = dirs.iter().map(|d| d.0.clone()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 16);
        // 1 つ消しても、他の一時ディレクトリは残る
        let first = dirs.remove(0);
        let first_path = first.0.clone();
        drop(first);
        assert!(!first_path.exists());
        assert!(dirs.iter().all(|d| d.0.exists()));
        drop(dirs);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// stdin を読まずに居座る偽の CLI。
    #[cfg(unix)]
    fn stuck_cli() -> (PathBuf, ProviderConfig) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kakiwake-stuck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stuck-cli");
        std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cfg = ProviderConfig {
            path: path.to_string_lossy().into_owned(),
            ..ProviderConfig::for_provider(Provider::Claude)
        };
        (dir, cfg)
    }

    /// パイプの容量（64 KiB）を超える本文
    #[cfg(unix)]
    fn big_prompt() -> Prompt {
        Prompt { system: "S".into(), user: "あ".repeat(200_000), tag: "T".into() }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_returns_even_if_cli_never_reads_stdin() {
        let (dir, cfg) = stuck_cli();
        let started = Instant::now();
        let r = run(Provider::Claude, &cfg, &big_prompt(), Duration::from_millis(500), None, None).await;
        assert!(matches!(r, Err(RunError::Timeout(_))), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_returns_even_if_cli_never_reads_stdin() {
        let (dir, cfg) = stuck_cli();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = tx.send(());
        });
        let started = Instant::now();
        let r = run(Provider::Claude, &cfg, &big_prompt(), Duration::from_secs(60), Some(rx), None).await;
        assert!(matches!(r, Err(RunError::Cancelled)), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_reasoning_refusal_keeps_request_id_out_of_log() {
        use std::os::unix::fs::PermissionsExt;

        let dir = WorkDir::new().unwrap();
        let path = dir.0.join("claude");
        let detail = "API Error: Sonnet 5.5's safeguards flagged this message\n\n\
            Details: `[reasoning_extraction]`\n\nRequest ID: req_test401";
        let result = serde_json::json!({ "type": "result", "is_error": true, "result": detail });
        std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\ncat <<'EOF'\n{result}\nEOF\nexit 1\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cfg = ProviderConfig {
            path: path.to_string_lossy().into_owned(),
            ..ProviderConfig::for_provider(Provider::Claude)
        };
        let prompt = crate::prompt::build("# Sample", crate::Format::Md, crate::Format::Md, None, None);
        let error = run(Provider::Claude, &cfg, &prompt, Duration::from_secs(5), None, None).await.unwrap_err();
        assert!(matches!(&error, RunError::ClaudeReasoningRefusal { .. }), "{error:?}");
        assert!(error.to_string().contains("Request ID: req_test401"));
        assert_eq!(error.log_summary(), "refusal: reasoning_extraction");
    }

    #[test]
    fn log_summary_never_contains_cli_output() {
        let secret = "社外秘の本文";
        let errs = [
            RunError::Failed { provider: "Copilot".into(), code: Some(1), stderr: format!("…{secret}") },
            RunError::Auth { provider: "Claude".into(), hint: "h".into(), detail: secret.into() },
            RunError::Empty("Codex".into(), secret.into()),
            RunError::ToolUse { provider: "Codex".into(), kind: "command_execution".into(), target: secret.into() },
        ];
        for e in &errs {
            assert!(!e.log_summary().contains(secret), "{}", e.log_summary());
            assert!(e.to_string().contains(secret) || matches!(e, RunError::ToolUse { .. }));
        }
        let patch =
            RunError::ToolUse { provider: "Codex".into(), kind: "file_change".into(), target: "/x/.bashrc".into() };
        assert_eq!(patch.log_summary(), "tool_use: file_change: /x/.bashrc");
    }

    #[test]
    fn detects_codex_tool_use() {
        let patch = r#"{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"/home/u/.bashrc","kind":"add"}],"status":"completed"}}"#;
        assert_eq!(codex_tool_use(patch), Some(("file_change".into(), "/home/u/.bashrc".into())));
        let cmd = r#"{"type":"item.started","item":{"id":"item_2","type":"command_execution","command":"bash -lc 'cat ~/.ssh/id_rsa'"}}"#;
        assert_eq!(codex_tool_use(cmd), Some(("command_execution".into(), "bash -lc 'cat ~/.ssh/id_rsa'".into())));
        let msg = r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"h1. T"}}"#;
        assert_eq!(codex_tool_use(msg), None);
        assert_eq!(codex_tool_use(r#"{"type":"turn.completed"}"#), None);
        assert_eq!(codex_tool_use("not json"), None);
    }

    #[test]
    fn parses_claude_stream() {
        let d = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"h1. T"}}}"#;
        assert_eq!(parse_claude_line(d), Some(ClaudeEvent::Delta("h1. T".into())));
        let r = r#"{"type":"result","subtype":"success","is_error":false,"result":"h1. T\n"}"#;
        assert_eq!(parse_claude_line(r), Some(ClaudeEvent::Result("h1. T\n".into())));
        let e = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
        assert_eq!(parse_claude_line(e), Some(ClaudeEvent::Error("Not logged in · Please run /login".into())));
        assert_eq!(parse_claude_line(r#"{"type":"system","subtype":"init"}"#), None);
        assert_eq!(parse_claude_line("not json"), None);
    }

    #[test]
    fn parses_registry_path() {
        let out = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    %USERPROFILE%\\bin;C:\\x\r\n";
        assert_eq!(parse_reg_path(out).as_deref(), Some("%USERPROFILE%\\bin;C:\\x"));
        unsafe { std::env::set_var("KAKIWAKE_TEST_DIR", "D:\\t") };
        assert_eq!(expand_env("%KAKIWAKE_TEST_DIR%\\a;%NOPE_X%;100%"), "D:\\t\\a;%NOPE_X%;100%");
    }

    #[test]
    fn parses_auth_status() {
        assert!(matches!(
            parse_auth(Provider::Claude, true, r#"{"loggedIn": true, "authMethod": "claude.ai"}"#, ""),
            AuthState::LoggedIn(m) if m == "claude.ai"
        ));
        assert!(matches!(parse_auth(Provider::Claude, true, r#"{"loggedIn": false}"#, ""), AuthState::LoggedOut(_)));
        assert!(matches!(parse_auth(Provider::Codex, true, "Logged in using ChatGPT", ""), AuthState::LoggedIn(_)));
        assert!(matches!(
            parse_auth(Provider::Codex, true, "", "Logged in using ChatGPT\n"),
            AuthState::LoggedIn(m) if m == "Logged in using ChatGPT"
        ));
        assert!(matches!(parse_auth(Provider::Codex, false, "", "Not logged in"), AuthState::LoggedOut(_)));
        assert!(looks_like_auth_error("Error: Not logged in. Please run /login"));
        assert!(!looks_like_auth_error("model not found"));
    }

    #[test]
    fn args_disable_tools() {
        let f = Path::new("/tmp/x");
        let c = build_args(Provider::Claude, &ProviderConfig::for_provider(Provider::Claude), f, "");
        assert!(c.windows(2).any(|w| w[0] == "--tools" && w[1].is_empty()));
        assert!(c.contains(&"claude-sonnet-5-5".to_string()));
        // どの引数にも改行を含めない（Windows の .cmd シム対策）
        assert!(c.iter().all(|a| !a.contains('\n')), "{c:?}");
        let x = build_args(Provider::Codex, &ProviderConfig::for_provider(Provider::Codex), f, "");
        assert!(x.windows(2).any(|w| w[0] == "--sandbox" && w[1] == "read-only"));
        assert_eq!(x.last().unwrap(), "-");
        assert!(x.contains(&"features.shell_tool=false".to_string()));
        assert!(x.contains(&"features.apps=false".to_string()));
        assert!(x.contains(&"--json".to_string()));
        let g = build_args(
            Provider::Copilot,
            &ProviderConfig::for_provider(Provider::Copilot),
            f,
            "--no-ask-user --deny-tool[=tools...]",
        );
        assert!(g.contains(&"--no-ask-user".to_string()));
        assert!(g.contains(&"--available-tools=none".to_string()));
        assert!(g.contains(&"--deny-tool=shell".to_string()));
        assert!(g.contains(&"--deny-tool=url".to_string()));
        assert!(!g.contains(&"--disable-client-telemetry".to_string()));
        assert!(!g.contains(&"-p".to_string()));
    }

    #[test]
    fn claude_uses_translation_preamble_in_command_and_stdin() {
        let prompt = crate::prompt::build(
            "Hello",
            crate::Format::Md,
            crate::Format::Md,
            None,
            Some(crate::TranslationLanguage::Japanese),
        );
        let preamble = prompt.claude_preamble();
        let args = build_args_with_preamble(
            Provider::Claude,
            &ProviderConfig::for_provider(Provider::Claude),
            Path::new("last-message.txt"),
            "",
            preamble,
        );
        assert!(preamble.contains("document translator"));
        assert!(!preamble.contains('\n'));
        assert!(args.windows(2).any(|w| w[0] == "--system-prompt" && w[1] == preamble));
        let payload = stdin_payload(&prompt);
        assert!(payload.starts_with(preamble));
        assert!(payload.contains("Output ONLY the translated document"));
    }
}
