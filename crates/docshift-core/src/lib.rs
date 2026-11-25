//! docshift のコアロジック（GUI に依存しない）。

pub mod detect;
pub mod format;
pub mod postprocess;
pub mod prompt;
pub mod provider;
pub mod quality;

pub use format::Format;
pub use provider::{Provider, ProviderConfig};

use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct Conversion {
    pub from: Format,
    pub to: Format,
    pub output: String,
    pub raw_output: String,
    pub elapsed_ms: u128,
    pub command: String,
    pub quality: Option<quality::QualityReport>,
}

pub struct ConvertOptions<'a> {
    pub provider: Provider,
    pub config: &'a ProviderConfig,
    pub timeout: Duration,
    pub extra_instructions: Option<&'a str>,
    pub quality_check: bool,
}

/// 1 回の変換（プロンプト生成 → CLI 実行 → 後処理 → 品質チェック）。
pub async fn convert(
    src: &str,
    from: Format,
    to: Format,
    opts: &ConvertOptions<'_>,
    cancel: Option<tokio::sync::oneshot::Receiver<()>>,
    on_text: Option<provider::OnText<'_>>,
) -> Result<Conversion, provider::RunError> {
    let prompt = prompt::build(src, from, to, opts.extra_instructions);
    let out = provider::run(opts.provider, opts.config, &prompt, opts.timeout, cancel, on_text).await?;
    let output = postprocess::clean(&out.text, src);
    let quality = opts.quality_check.then(|| quality::check(src, from, &output, to));
    Ok(Conversion { from, to, output, raw_output: out.text, elapsed_ms: out.elapsed_ms, command: out.command, quality })
}
