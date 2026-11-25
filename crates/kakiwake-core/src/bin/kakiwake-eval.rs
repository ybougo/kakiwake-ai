//! 実際の CLI で変換し、品質チェックの結果を一覧にする評価ツール。
//!
//! cargo run -p kakiwake-core --bin kakiwake-eval -- \
//!     [--providers claude,codex,copilot] [--model codex=gpt-6.2-sol] [--jobs 3] [--out eval-out] [--fixtures DIR]
//!     [--cases readme.md]   # ファイル名か「ファイル名→形式」の部分一致で絞り込む（カンマ区切り）
//!     [--recheck DIR]       # LLM を呼ばず、DIR に保存済みの出力へ品質チェックだけをかけ直す

use kakiwake_core::quality::Grade;
use kakiwake_core::{ConvertOptions, Format, Provider, ProviderConfig, convert};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

const CASES: &[(&str, Format, Format)] = &[
    ("sample.md", Format::Md, Format::Adoc),
    ("sample.md", Format::Md, Format::Textile),
    ("sample.md", Format::Md, Format::Plain),
    ("sample.txt", Format::Plain, Format::Md),
    ("sample.txt", Format::Plain, Format::Adoc),
    ("sample.txt", Format::Plain, Format::Textile),
    ("sample.textile", Format::Textile, Format::Md),
    ("sample.textile", Format::Textile, Format::Adoc),
    ("sample.textile", Format::Textile, Format::Plain),
    ("readme.md", Format::Md, Format::Textile),
    ("readme.md", Format::Md, Format::Adoc),
];

struct Row {
    provider: Provider,
    model: String,
    case: String,
    grade: String,
    score: u32,
    recall: f64,
    precision: f64,
    secs: f64,
    /// 最初の途中出力が届くまでの秒数（途中出力のない CLI は None）
    first: Option<f64>,
    notes: String,
}

#[tokio::main]
async fn main() {
    let mut providers = Provider::ALL.to_vec();
    let mut models: HashMap<Provider, String> = HashMap::new();
    let mut jobs = 3usize;
    let mut out_dir = PathBuf::from("eval-out");
    let mut fixtures = PathBuf::from(FIXTURES);
    let mut filters: Vec<String> = Vec::new();
    let mut recheck: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--providers" => {
                providers = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|s| serde_json::from_str::<Provider>(&format!("\"{}\"", s.trim())).ok())
                    .collect()
            }
            "--model" => {
                let v = args.next().unwrap_or_default();
                if let Some((p, m)) = v.split_once('=')
                    && let Ok(p) = serde_json::from_str::<Provider>(&format!("\"{p}\""))
                {
                    models.insert(p, m.to_string());
                }
            }
            "--jobs" => jobs = args.next().and_then(|s| s.parse().ok()).unwrap_or(3),
            "--out" => out_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--fixtures" => fixtures = PathBuf::from(args.next().unwrap_or_default()),
            "--recheck" => recheck = args.next().map(PathBuf::from),
            "--cases" => filters = args.next().unwrap_or_default().split(',').map(|s| s.trim().to_string()).collect(),
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    if let Some(dir) = recheck {
        recheck_dir(&dir, &fixtures);
        return;
    }

    let mut handles = Vec::new();
    for p in providers {
        let mut cfg = ProviderConfig::for_provider(p);
        if let Some(m) = models.get(&p) {
            cfg.model = m.clone();
        }
        let sem = Arc::new(Semaphore::new(jobs));
        let dir = out_dir.join(format!("{}-{}", p.id(), cfg.model));
        std::fs::create_dir_all(&dir).expect("create out dir");
        let selected = CASES.iter().filter(|(file, _, to)| {
            let name = format!("{file}→{to}");
            filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()))
        });
        for (file, from, to) in selected {
            let cfg = cfg.clone();
            let sem = sem.clone();
            let dir = dir.clone();
            let fixtures = fixtures.clone();
            let (file, from, to) = (*file, *from, *to);
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                run_case(p, cfg, &fixtures, file, from, to, &dir).await
            }));
        }
    }

    let mut rows = Vec::new();
    for h in handles {
        rows.push(h.await.unwrap());
    }
    rows.sort_by(|a, b| (a.provider.id(), &a.case).cmp(&(b.provider.id(), &b.case)));

    let mut md = String::from(
        "| provider | model | case | grade | score | recall | precision | sec | first | notes |\n|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.3} | {:.3} | {:.1} | {} | {} |\n",
            r.provider.id(),
            r.model,
            r.case,
            r.grade,
            r.score,
            r.recall,
            r.precision,
            r.secs,
            r.first.map(|f| format!("{f:.1}")).unwrap_or_else(|| "-".into()),
            r.notes.replace('|', "\\|")
        ));
    }
    std::fs::write(out_dir.join("report.md"), &md).expect("write report");
    println!("{md}");
}

async fn run_case(
    p: Provider,
    cfg: ProviderConfig,
    fixtures: &std::path::Path,
    file: &str,
    from: Format,
    to: Format,
    dir: &std::path::Path,
) -> Row {
    let src = std::fs::read_to_string(fixtures.join(file)).expect("fixture");
    let case = format!("{file}→{to}");
    let opts = ConvertOptions {
        provider: p,
        config: &cfg,
        timeout: Duration::from_secs(300),
        extra_instructions: None,
        translation: None,
        quality_check: true,
    };
    let started = std::time::Instant::now();
    let first = std::sync::Mutex::new(None::<f64>);
    let on_text = |_: &str| {
        let mut f = first.lock().unwrap();
        if f.is_none() {
            *f = Some(started.elapsed().as_secs_f64());
        }
    };
    let result = convert(&src, from, to, &opts, None, Some(&on_text)).await;
    let first = *first.lock().unwrap();
    match result {
        Ok(c) => {
            let q = c.quality.unwrap();
            let stem = file.replace('.', "_");
            let _ = std::fs::write(dir.join(format!("{stem}.{to}.out")), &c.output);
            let _ = std::fs::write(
                dir.join(format!("{stem}.{to}.quality.json")),
                serde_json::to_string_pretty(&q).unwrap(),
            );
            Row {
                provider: p,
                model: cfg.model.clone(),
                case,
                grade: match q.grade {
                    Grade::Ok => "OK",
                    Grade::Warn => "WARN",
                    Grade::Ng => "NG",
                }
                .into(),
                score: q.score,
                recall: q.recall,
                precision: q.precision,
                secs: c.elapsed_ms as f64 / 1000.0,
                first,
                notes: q.messages.join(" "),
            }
        }
        Err(e) => Row {
            provider: p,
            model: cfg.model.clone(),
            case,
            grade: "ERROR".into(),
            score: 0,
            recall: 0.0,
            precision: 0.0,
            secs: 0.0,
            first: None,
            notes: e.to_string().replace('\n', " "),
        },
    }
}

/// 保存済みの出力（<provider>-<model>/<stem>.<to>.out）に品質チェックをかけ直す。
fn recheck_dir(dir: &std::path::Path, fixtures: &std::path::Path) {
    let mut entries: Vec<_> =
        std::fs::read_dir(dir).expect("recheck dir").flatten().filter(|e| e.path().is_dir()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        for (file, from, to) in CASES {
            let out = e.path().join(format!("{}.{to}.out", file.replace('.', "_")));
            let Ok(output) = std::fs::read_to_string(&out) else { continue };
            let src = std::fs::read_to_string(fixtures.join(file)).expect("fixture");
            let q = kakiwake_core::quality::check(&src, *from, &output, *to);
            println!(
                "| {} | {file}→{to} | {:?} | {} | {} |",
                e.file_name().to_string_lossy(),
                q.grade,
                q.score,
                q.messages.join(" ")
            );
        }
    }
}
