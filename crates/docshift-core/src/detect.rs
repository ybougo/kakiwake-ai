use crate::format::Format;
use regex::Regex;
use std::sync::LazyLock;

struct Rule {
    re: Regex,
    format: Format,
    weight: u32,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |p: &str, format: Format, weight: u32| Rule { re: Regex::new(p).unwrap(), format, weight };
    vec![
        // Textile
        r(r"(?m)^h[1-6]\.\s", Format::Textile, 5),
        r(r"(?m)^bq\.\s", Format::Textile, 4),
        r(r"(?m)^\|_\.", Format::Textile, 5),
        r(r"<pre>|</pre>", Format::Textile, 3),
        r(r"\{\{toc\}\}", Format::Textile, 5),
        r(r#""[^"\n]+":https?://"#, Format::Textile, 4),
        r(r"@[^@\s][^@\n]*@", Format::Textile, 1),
        // AsciiDoc
        r(r"(?m)^={1,5} \S", Format::Adoc, 4),
        r(r"(?m)^\|===\s*$", Format::Adoc, 6),
        r(r"(?m)^\[source(,[^\]]*)?\]\s*$", Format::Adoc, 6),
        r(r"(?m)^----\s*$", Format::Adoc, 2),
        r(r"(?m)^:[A-Za-z0-9_-]+:", Format::Adoc, 3),
        r(r"(?m)^\.{1,3} \S", Format::Adoc, 2),
        r(r"https?://\S+\[[^\]]*\]", Format::Adoc, 4),
        r(r"(?m)^(NOTE|TIP|WARNING|IMPORTANT|CAUTION): ", Format::Adoc, 3),
        // Markdown
        r(r"(?m)^#{1,6} \S", Format::Md, 4),
        r(r"(?m)^```", Format::Md, 3),
        r(r"\[[^\]\n]+\]\([^)\s]+\)", Format::Md, 4),
        r(r"\*\*[^*\n]+\*\*", Format::Md, 2),
        r(r"(?m)^\|.*\|\s*$\n^\|\s*:?-{3,}", Format::Md, 6),
        r(r"(?m)^\s*[-*+] \[[ xX]\] ", Format::Md, 3),
        r(r"(?m)^\s*- \S", Format::Md, 1),
        r(r"(?m)^> ", Format::Md, 1),
    ]
});

/// 入力の形式を推定する。どの記法の特徴も弱ければ Plain。
pub fn detect(text: &str) -> Format {
    let mut scores = [0u32; Format::ALL.len()];
    let idx = |f: Format| Format::ALL.iter().position(|x| *x == f).unwrap();
    for rule in RULES.iter() {
        let hits = rule.re.find_iter(text).take(20).count() as u32;
        scores[idx(rule.format)] += hits * rule.weight;
    }
    let (best_i, best) = scores.iter().enumerate().max_by_key(|(_, s)| **s).unwrap();
    if *best < 3 {
        return Format::Plain;
    }
    Format::ALL[best_i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_format() {
        assert_eq!(detect("# 見出し\n\n- a\n- b\n\n[link](https://x.y)"), Format::Md);
        assert_eq!(detect("= Title\n\n== 概要\n\n* a\n\n[source,sh]\n----\nls\n----"), Format::Adoc);
        assert_eq!(detect("h1. Title\n\n* a\n# b\n\n\"link\":https://x.y"), Format::Textile);
        assert_eq!(detect("会議メモ\n\n・参加者 田中、鈴木\n・次回 12/1"), Format::Plain);
    }
}
