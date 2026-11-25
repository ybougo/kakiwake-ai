use regex::Regex;
use std::sync::LazyLock;

static PREFACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(here\s+is|here's|below\s+is|sure[,!]|certainly[,!]|converted|the\s+converted|以下(は|が|に)|変換(結果|しました|後)|承知しました|了解しました)",
    )
    .unwrap()
});
static TRANSLATION_PREFACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)^(?:
            (?:(?:sure|certainly|of\s+course)[!,]\s+)?
            (?:here\s+is|here['’]s|below\s+is|the\s+following\s+is)\s+(?:(?:the|your)\s+)?
            (?:(?:english|japanese)\s+)?(?:translation|translated\s+(?:document|text|version))
            (?:\s+of\s+(?:the|your)\s+(?:document|text))?(?:\s+in\s+(?:english|japanese))?
            |(?:(?:以下|こちら)(?:は|が)\s*)?(?:翻訳(?:結果|文)|訳文|(?:日本語|英語)訳)(?:です)
        )\s*[:：]$",
    )
    .unwrap()
});
static FENCE_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(```+|~~~+)[A-Za-z0-9_+.\-]*\s*$").unwrap());

/// LLM の出力から変換結果以外の付随物を取り除く。
/// `source` は「元から全体がコードブロックだった」等の誤除去を避けるために使う。
pub fn clean(raw: &str, source: &str, tag: &str) -> String {
    clean_output(raw, source, tag, false)
}

/// 翻訳結果の前置きは、明示的なラベルだけを除く。
pub fn clean_translation(raw: &str, source: &str, tag: &str) -> String {
    clean_output(raw, source, tag, true)
}

fn clean_output(raw: &str, source: &str, tag: &str, translation: bool) -> String {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.lines().collect();

    // 区切りタグのエコーを除去する。今回使ったタグが出力の先頭・末尾にあるときだけ外す
    // （本文やコードブロック内の同じ文字列は元の文書の内容なので残す）
    strip_outer_tag(&mut lines, tag);
    trim_blank(&mut lines);

    // 先頭の前置き文（「以下が変換結果です:」など）＋空行 を除去
    if lines.len() > 2 && (lines[1].trim().is_empty() || FENCE_OPEN.is_match(lines[1].trim())) {
        let first = lines[0].trim();
        let looks_preface = if translation {
            TRANSLATION_PREFACE.is_match(first)
        } else {
            PREFACE.is_match(first) && first.ends_with([':', '：', '。', '.'])
        };
        if looks_preface && !source.trim_start().starts_with(first) {
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
    // 前置き文や全体を囲むフェンスの内側にタグがある場合
    strip_outer_tag(&mut lines, tag);

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// 今回使った区切りタグが先頭・末尾にあれば外す。
fn strip_outer_tag(lines: &mut Vec<&str>, tag: &str) {
    trim_blank(lines);
    if lines.first().is_some_and(|l| l.trim() == format!("<{tag}>")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == format!("</{tag}>")) {
        lines.pop();
    }
    trim_blank(lines);
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

    const TAG: &str = "KAKIWAKE_INPUT";

    #[test]
    fn strips_wrapping_fence() {
        assert_eq!(clean("```asciidoc\n= T\n\nbody\n```\n", "# T\n\nbody", TAG), "= T\n\nbody\n");
    }

    #[test]
    fn keeps_fence_when_source_is_code() {
        let s = "```\ncode\n```";
        assert_eq!(clean(s, s, TAG), "```\ncode\n```\n");
    }

    #[test]
    fn keeps_inner_code_blocks() {
        let s = "```\na\n```\n\ntext\n\n```\nb\n```";
        assert_eq!(clean(s, "x", TAG), format!("{s}\n"));
    }

    #[test]
    fn strips_preface() {
        assert_eq!(clean("以下が変換結果です。\n\nh1. T\n\nbody", "# T\n\nbody", TAG), "h1. T\n\nbody\n");
        assert_eq!(clean("Here is the converted document:\n```\n= T\nx\n```", "# T\nx", TAG), "= T\nx\n");
    }

    #[test]
    fn keeps_translated_first_sentence() {
        let output = "Here is the overview.\n\nSee the following steps.\n";
        assert_eq!(clean_translation(output, "概要は次のとおりです。", TAG), output);
    }

    #[test]
    fn strips_translation_label_before_blank_line_or_fence() {
        assert_eq!(clean_translation("Here is the translation:\n\n# Hello", "# こんにちは", TAG), "# Hello\n");
        assert_eq!(
            clean_translation("Sure! Here's the Japanese translation of the document:\n\nこんにちは", "Hello", TAG),
            "こんにちは\n"
        );
        assert_eq!(clean_translation("以下は日本語訳です：\n\nこんにちは", "Hello", TAG), "こんにちは\n");
        assert_eq!(
            clean_translation("以下は翻訳結果です：\n```md\n# こんにちは\n```", "# Hello", TAG),
            "# こんにちは\n"
        );
    }

    #[test]
    fn keeps_translated_headings_and_inline_labels() {
        for output in [
            "Here is the overview:\n\nThe first step.",
            "Translation:\n\nAn example.",
            "翻訳結果：\n\n本文です。",
            "Here is the translation:\nAn example.",
        ] {
            assert_eq!(clean_translation(output, "原文", TAG), format!("{output}\n"));
        }
    }

    #[test]
    fn keeps_tag_like_lines_inside_content() {
        // 元の文書にタグと同じ文字列があっても（入力側は別名のタグを使う）、本文の行は消さない
        let out = "h1. T\n\n<pre>\n<KAKIWAKE_INPUT>\n</KAKIWAKE_INPUT>\n</pre>\n";
        assert_eq!(clean(out, "x", "KAKIWAKE_INPUT_1"), out);
        assert_eq!(clean(&format!("<KAKIWAKE_INPUT_1>\n{out}</KAKIWAKE_INPUT_1>"), "x", "KAKIWAKE_INPUT_1"), out);
    }

    #[test]
    fn strips_echoed_tags() {
        assert_eq!(clean("以下が変換結果です:\n\n<KAKIWAKE_INPUT>\nh1. T\n</KAKIWAKE_INPUT>", "# T", TAG), "h1. T\n");
        assert_eq!(clean("<KAKIWAKE_INPUT>\nh1. T\n</KAKIWAKE_INPUT>", "# T", TAG), "h1. T\n");
    }
}
