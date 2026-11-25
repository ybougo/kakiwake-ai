use kakiwake_core::{Conversion, ConvertOptions, Format, Provider, ProviderConfig, detect, provider, quality};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: Provider,
    pub providers: HashMap<Provider, ProviderConfig>,
    /// プロバイダごとに GUI で追加したモデル ID（候補に出す）
    pub custom_models: HashMap<Provider, Vec<String>>,
    pub timeout_sec: u64,
    pub quality_check: bool,
    /// 品質チェックが NG のとき 1 回だけ自動で再変換する
    pub retry_on_ng: bool,
    /// 変換先ごとの追加指示
    pub extra_instructions: HashMap<Format, String>,
    pub last_from: Option<Format>,
    pub last_to: Format,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Claude,
            providers: Provider::ALL.iter().map(|p| (*p, ProviderConfig::for_provider(*p))).collect(),
            custom_models: HashMap::new(),
            timeout_sec: 180,
            quality_check: true,
            retry_on_ng: false,
            extra_instructions: HashMap::new(),
            last_from: None,
            last_to: Format::Md,
        }
    }
}

impl Settings {
    fn normalized(mut self) -> Self {
        for p in Provider::ALL {
            let cfg = self.providers.entry(p).or_insert_with(|| ProviderConfig::for_provider(p));
            if cfg.model.trim().is_empty() {
                cfg.model = p.default_model().into();
            }
        }
        self.timeout_sec = self.timeout_sec.clamp(10, 1800);
        self
    }
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    settings_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default()
        .normalized()
}

// ---------------------------------------------------------------------------
// 状態
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Jobs(Mutex<HashMap<u64, oneshot::Sender<()>>>);

// ---------------------------------------------------------------------------
// コマンド
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ModelPresetDto {
    id: String,
    label: String,
    note: String,
}

#[derive(Serialize)]
struct ProviderInfo {
    id: Provider,
    label: &'static str,
    default_model: &'static str,
    presets: Vec<ModelPresetDto>,
    efforts: Vec<&'static str>,
}

#[tauri::command]
fn provider_catalog() -> Vec<ProviderInfo> {
    Provider::ALL
        .iter()
        .map(|p| ProviderInfo {
            id: *p,
            label: p.label(),
            default_model: p.default_model(),
            presets: p
                .presets()
                .iter()
                .map(|m| ModelPresetDto { id: m.id.into(), label: m.label.into(), note: m.note.into() })
                .collect(),
            efforts: p.efforts().to_vec(),
        })
        .collect()
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Settings {
    load_settings(&app)
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<Settings, String> {
    let settings = settings.normalized();
    let path = settings_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[derive(Serialize)]
struct ProviderStatus {
    id: Provider,
    ok: bool,
    path: Option<String>,
    version: Option<String>,
    error: Option<String>,
    auth: Option<provider::AuthState>,
}

#[tauri::command]
async fn check_providers(app: tauri::AppHandle) -> Vec<ProviderStatus> {
    let settings = load_settings(&app);
    let checks = Provider::ALL.map(|p| {
        let cfg = settings.providers.get(&p).cloned().unwrap_or_else(|| ProviderConfig::for_provider(p));
        async move {
            let path = provider::resolve(p, &cfg).ok().map(|x| x.to_string_lossy().into_owned());
            match provider::version(p, &cfg).await {
                Ok(v) => {
                    let auth = provider::auth_status(p, &cfg)
                        .await
                        .unwrap_or_else(|e| provider::AuthState::Unknown(e.to_string()));
                    let logged_out = matches!(auth, provider::AuthState::LoggedOut(_));
                    ProviderStatus {
                        id: p,
                        ok: !logged_out,
                        path,
                        version: Some(v),
                        error: logged_out.then(|| format!("ログインしていません。{}", p.login_hint())),
                        auth: Some(auth),
                    }
                }
                Err(e) => {
                    ProviderStatus { id: p, ok: false, path, version: None, error: Some(e.to_string()), auth: None }
                }
            }
        }
    });
    let [a, b, c] = checks;
    let (a, b, c) = tokio::join!(a, b, c);
    vec![a, b, c]
}

/// ログイン用のターミナルを開く（ログインはブラウザ／デバイスコード認証のため CLI 自身に任せる）。
#[tauri::command]
fn open_login(app: tauri::AppHandle, provider: Provider) -> Result<(), String> {
    let settings = load_settings(&app);
    let cfg = settings.providers.get(&provider).cloned().unwrap_or_else(|| ProviderConfig::for_provider(provider));
    let exe = provider::resolve(provider, &cfg).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        // 新しいコンソールでログインコマンドを実行し、終わってもウィンドウを残す
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/c").arg("start").arg("Kakiwake AI login").arg("cmd.exe").arg("/k").arg(&exe);
        cmd.args(provider.login_args());
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Err(format!("ターミナルで次を実行してください: {} {}", exe.to_string_lossy(), provider.login_args().join(" ")))
    }
}

#[derive(Deserialize)]
struct ConvertRequest {
    job_id: u64,
    source: String,
    /// None なら自動判定
    from: Option<Format>,
    to: Format,
    provider: Provider,
    model: String,
    effort: Option<String>,
}

#[derive(Serialize)]
struct ConvertResponse {
    #[serde(flatten)]
    conversion: Conversion,
    detected_from: bool,
    provider: Provider,
    model: String,
    retried: bool,
}

/// 生成途中の出力を画面へ送るメッセージ。
#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
enum StreamMsg {
    /// 再変換などで、それまでの途中出力を捨てる
    Reset,
    Text {
        text: String,
    },
}

#[tauri::command]
async fn convert(
    app: tauri::AppHandle,
    jobs: State<'_, Jobs>,
    req: ConvertRequest,
    on_stream: Channel<StreamMsg>,
) -> Result<ConvertResponse, String> {
    if req.source.trim().is_empty() {
        return Err("入力が空です。".into());
    }
    let settings = load_settings(&app);
    let mut cfg =
        settings.providers.get(&req.provider).cloned().unwrap_or_else(|| ProviderConfig::for_provider(req.provider));
    if !req.model.trim().is_empty() {
        cfg.model = req.model.trim().to_string();
    }
    if let Some(e) = req.effort {
        cfg.effort = e;
    }
    let (from, detected_from) = match req.from {
        Some(f) => (f, false),
        None => (detect::detect(&req.source), true),
    };
    let extra = settings.extra_instructions.get(&req.to).cloned();
    let opts = ConvertOptions {
        provider: req.provider,
        config: &cfg,
        timeout: Duration::from_secs(settings.timeout_sec),
        extra_instructions: extra.as_deref(),
        quality_check: settings.quality_check || settings.retry_on_ng,
    };

    let mut retried = false;
    let mut attempt = 0;
    let conversion = loop {
        let (tx, rx) = oneshot::channel();
        jobs.0.lock().unwrap().insert(req.job_id, tx);
        if attempt > 0 {
            let _ = on_stream.send(StreamMsg::Reset);
        }
        let send = |t: &str| {
            let _ = on_stream.send(StreamMsg::Text { text: t.to_string() });
        };
        let r = kakiwake_core::convert(&req.source, from, req.to, &opts, Some(rx), Some(&send)).await;
        jobs.0.lock().unwrap().remove(&req.job_id);
        let c = r.map_err(|e| e.to_string())?;
        let is_ng = c.quality.as_ref().is_some_and(|q| q.grade == quality::Grade::Ng);
        if settings.retry_on_ng && is_ng && attempt == 0 {
            attempt += 1;
            retried = true;
            continue;
        }
        break c;
    };
    let mut conversion = conversion;
    if !settings.quality_check {
        conversion.quality = None;
    }
    Ok(ConvertResponse { conversion, detected_from, provider: req.provider, model: cfg.model, retried })
}

#[tauri::command]
fn cancel(jobs: State<'_, Jobs>, job_id: u64) -> bool {
    jobs.0.lock().unwrap().remove(&job_id).is_some_and(|tx| tx.send(()).is_ok())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Jobs::default())
        .invoke_handler(tauri::generate_handler![
            provider_catalog,
            get_settings,
            save_settings,
            check_providers,
            open_login,
            convert,
            cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running kakiwake-ai");
}
