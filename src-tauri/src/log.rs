//! ローカルのログファイル（外部には送らない）。
//!
//! 保存先: `%LOCALAPPDATA%\dev.kakiwake-ai.app\logs\kakiwake-ai.log`
//! 1 MB を超えたら `kakiwake-ai.log.1` に退避して作り直す。文書の本文は書かない（文字数のみ）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const MAX_BYTES: u64 = 1024 * 1024;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("logs").join("kakiwake-ai.log"))
}

/// 1 行追記する。ログの失敗でアプリを止めないよう、エラーは無視する。
pub fn write(app: &tauri::AppHandle, message: &str) {
    let Some(path) = path(app) else { return };
    let _guard = WRITE_LOCK.lock();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        // 改行を含むエラーは 1 行にまとめる
        let one_line = message.replace("\r\n", " ⏎ ").replace('\n', " ⏎ ");
        let _ = writeln!(f, "{} {one_line}", timestamp());
    }
}

/// UTC の ISO 8601 形式（例: 2026-11-25T02:30:00Z）。日付計算だけなので外部 crate は使わない。
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    format_utc(secs)
}

fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::format_utc;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_795_573_800), "2026-11-25T02:30:00Z");
    }
}
