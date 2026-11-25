use regex::Regex;
use std::sync::LazyLock;

static PREFACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(here\s+is|here's|below\s+is|sure[,!]|certainly[,!]|converted|the\s+converted|以下(は|が|に)|変換(結果|しました|後)|承知しました|了解しました)",
    )
    .unwrap()
});
static FENCE_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(```+|~~~+)[A-Za-z0-9_+.\-]*\s*$").unwrap());
static TAG_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^</?DOCSHIFT_INPUT(_\d+)?>\s*$").unwrap());

/// LLM の出力から変換結果以外の付随物を取り除く。
/// `source` は「元から全体がコードブロックだった」等の誤除去を避けるために使う。
pub fn clean(raw: &str, source: &str) -> String {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.lines().collect();

    // 区切りタグのエコーを除去
    lines.retain(|l| !TAG_LINE.is_match(l.trim()));
    trim_blank(&mut lines);

    // 先頭の前置き文（「以下が変換結果です:」など）＋空行 を除去
    if lines.len() > 2 {
        let first = lines[0].trim();
        let looks_preface = PREFACE.is_match(first)
            && (first.ends_with(':') || first.ends_with('：') || first.ends_with('。') || first.ends_with('.'))
            && !source.trim_start().starts_with(first);
        if looks_preface && (lines[1].trim().is_empty() || FENCE_OPEN.is_match(lines[1].trim())) {
            lines.remove(0);
            trim_blank(&mut lines);
        }
    }

    // 全体を囲むコードフェンスを除去（元文書自体がフェンスで始まる場合は残す）
    let source_starts_with_fence = source.trim_start().lines().next().is_some_and(|l| FENCE_OPEN.is_match(l.trim()));
    if !source_starts_with_fence && lines.len() >= 2 {
        let first = lines[0].trim();
        let last = lines[lines.len() - 1].trim();
        if let Some(m) = FENCE_OPEN.find(first) {
            let fence: String = m.as_str().chars().take_while(|c| *c == '`' || *c == '~').collect();
            if last == fence {
                // 内側で外側フェンスが先に閉じてしまうなら、全体囲みではない
                let inner = &lines[1..lines.len() - 1];
                let mut depth = 0usize;
                let mut closed_early = false;
                for l in inner {
                    let t = l.trim();
                    if !t.starts_with(&fence) {
                        continue;
                    }
                    if t == fence {
                        if depth == 0 {
                            closed_early = true;
                            break;
                        }
                        depth -= 1;
                    } else if FENCE_OPEN.is_match(t) {
                        depth += 1;
                    }
                }
                if !closed_early {
                    lines.remove(lines.len() - 1);
                    lines.remove(0);
                }
            }
        }
    }
    trim_blank(&mut lines);

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn trim_blank(lines: &mut Vec<&str>) {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_wrapping_fence() {
        assert_eq!(clean("```asciidoc\n= T\n\nbody\n```\n", "# T\n\nbody"), "= T\n\nbody\n");
    }

    #[test]
    fn keeps_fence_when_source_is_code() {
        let s = "```\ncode\n```";
        assert_eq!(clean(s, s), "```\ncode\n```\n");
    }

    #[test]
    fn keeps_inner_code_blocks() {
        let s = "```\na\n```\n\ntext\n\n```\nb\n```";
        assert_eq!(clean(s, "x"), format!("{s}\n"));
    }

    #[test]
    fn strips_preface() {
        assert_eq!(clean("以下が変換結果です。\n\nh1. T\n\nbody", "# T\n\nbody"), "h1. T\n\nbody\n");
        assert_eq!(clean("Here is the converted document:\n```\n= T\nx\n```", "# T\nx"), "= T\nx\n");
    }

    #[test]
    fn strips_echoed_tags() {
        assert_eq!(clean("<DOCSHIFT_INPUT>\nh1. T\n</DOCSHIFT_INPUT>", "# T"), "h1. T\n");
    }
}
