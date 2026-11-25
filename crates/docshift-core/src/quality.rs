//! 変換結果の品質チェック（決定的・ローカル完結。LLM は使わない）。
//!
//! 1. 内容保持: 記法を除去して正規化した本文の文字 bigram で再現率 / 適合率を測る
//! 2. 欠落検出: 元の行・URL・数値が出力に残っているか
//! 3. 構造比較: 見出し・リスト項目・表セル・コードブロックの数
//! 4. 記法チェック: 出力に変換先以外の記法が混入していないか

use crate::format::Format;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Ok,
    Warn,
    Ng,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructureDiff {
    pub kind: &'static str,
    pub label: &'static str,
    pub source: usize,
    pub output: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyntaxIssue {
    pub line: usize,
    pub text: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QualityReport {
    pub grade: Grade,
    /// 0-100 の総合スコア
    pub score: u32,
    /// 元の本文がどれだけ出力に残っているか（0.0-1.0）
    pub recall: f64,
    /// 出力の本文がどれだけ元に由来するか（0.0-1.0）
    pub precision: f64,
    pub missing_lines: Vec<String>,
    pub added_lines: Vec<String>,
    pub missing_urls: Vec<String>,
    pub missing_numbers: Vec<String>,
    pub structure: Vec<StructureDiff>,
    pub syntax_issues: Vec<SyntaxIssue>,
    /// 画面表示用の要約（日本語）
    pub messages: Vec<String>,
}

const MAX_LIST: usize = 20;

pub fn check(source: &str, from: Format, output: &str, to: Format) -> QualityReport {
    let src_doc = Stripped::new(source, from);
    let out_doc = Stripped::new(output, to);

    let src_norm = normalize(&src_doc.text);
    let out_norm = normalize(&out_doc.text);
    // 行をまたぐ bigram は数えない（表 → 箇条書きなど、行の組み替えに強くする）
    let src_bi = line_bigrams(&src_doc.lines);
    let out_bi = line_bigrams(&out_doc.lines);
    let common = intersection(&src_bi, &out_bi);
    let src_total: usize = src_bi.values().sum();
    let out_total: usize = out_bi.values().sum();
    let recall = ratio(common, src_total);
    let precision = ratio(common, out_total);

    let missing_lines = unmatched_lines(&src_doc.lines, &out_norm, &out_bi);
    let added_lines = unmatched_lines(&out_doc.lines, &src_norm, &src_bi);
    let missing_urls = missing_urls(source, output);
    let missing_numbers = missing_numbers(&src_doc.text, &out_doc.text);
    let structure = compare_structure(source, from, output, to);
    let syntax_issues = lint(output, to);

    let precision_floor = 0.85;
    let mut messages = Vec::new();
    if output.trim().is_empty() {
        messages.push("出力が空です。".to_string());
    }
    if recall < 0.97 {
        messages.push(format!("元の内容の {:.1}% しか出力に残っていません。", recall * 100.0));
    }
    if precision < precision_floor {
        messages.push(format!(
            "出力の {:.1}% が元の文書にない内容です（追加・言い換えの可能性）。",
            (1.0 - precision) * 100.0
        ));
    }
    if !missing_lines.is_empty() {
        messages.push(format!("欠落の可能性がある行が {} 行あります。", missing_lines.len()));
    }
    if !added_lines.is_empty() {
        messages.push(format!("元の文書にない行が {} 行あります（注釈・説明文の混入の可能性）。", added_lines.len()));
    }
    if !missing_urls.is_empty() {
        messages.push(format!("URL が {} 件欠落しています。", missing_urls.len()));
    }
    if !missing_numbers.is_empty() {
        messages.push(format!("数値が {} 件欠落・変化しています。", missing_numbers.len()));
    }
    for d in &structure {
        messages.push(format!("{}の数が違います（元 {} → 出力 {}）。", d.label, d.source, d.output));
    }
    if !syntax_issues.is_empty() {
        messages.push(format!("変換先にない記法が {} 箇所あります。", syntax_issues.len()));
    }

    let ng = output.trim().is_empty() || recall < 0.85 || precision < precision_floor - 0.15 || missing_urls.len() >= 3;
    let warn = !messages.is_empty();
    let grade = if ng {
        Grade::Ng
    } else if warn {
        Grade::Warn
    } else {
        Grade::Ok
    };

    let mut score = 100.0 * (0.7 * recall + 0.3 * (precision / precision_floor).min(1.0));
    score -= 3.0 * missing_urls.len() as f64;
    score -= 2.0 * missing_numbers.len().min(5) as f64;
    score -= 2.0 * structure.len() as f64;
    score -= 1.0 * syntax_issues.len().min(10) as f64;
    let score = score.clamp(0.0, 100.0).round() as u32;

    QualityReport {
        grade,
        score,
        recall,
        precision,
        missing_lines: truncate(missing_lines),
        added_lines: truncate(added_lines),
        missing_urls: truncate(missing_urls),
        missing_numbers: truncate(missing_numbers),
        structure,
        syntax_issues: syntax_issues.into_iter().take(MAX_LIST).collect(),
        messages,
    }
}

fn truncate(mut v: Vec<String>) -> Vec<String> {
    v.truncate(MAX_LIST);
    v
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 1.0 } else { a as f64 / b as f64 }
}

// ---------------------------------------------------------------------------
// 記法の除去
// ---------------------------------------------------------------------------

/// 記法を取り除いた本文。`lines` は空でない行（行単位の欠落検出用）。
struct Stripped {
    text: String,
    lines: Vec<String>,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(MD_FENCE, r"^\s*(```+|~~~+)");
re!(MD_HEADING, r"^#{1,6}\s+");
re!(MD_LIST, r"^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?");
re!(MD_QUOTE, r"^\s*(?:>\s?)+");
re!(MD_TABLE_SEP, r"^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$");
re!(MD_HR, r"^\s*([-*_])(\s*[-*_]){2,}\s*$");
re!(MD_IMAGE, r"!\[([^\]]*)\]\(([^)\s]+)[^)]*\)");
re!(MD_LINK, r"\[([^\]]+)\]\(([^)\s]+)[^)]*\)");
re!(MD_EMPH, r"\*\*|__|~~|(^|[\s(])[*_]|[*_]([\s).,:;!?]|$)");
re!(BACKTICK, r"`+");

re!(ADOC_DELIM, r"^(----+|\.\.\.\.+|====+|____+|\*\*\*\*+|\+\+\+\++|--)\s*$");
re!(ADOC_CODE_DELIM, r"^(----+|\.\.\.\.+|\+\+\+\++)\s*$");
re!(ADOC_HEADING, r"^={1,6}\s+");
re!(ADOC_LIST, r"^\s*(?:\*+|\.+|-)\s+(?:\[[ xX*]\]\s+)?");
re!(ADOC_ATTR_LINE, r"^\s*(\[[^\]]*\]|:[A-Za-z0-9_-]+:.*|//.*|'''+|\+)\s*$");
re!(ADOC_TABLE_DELIM, r"^\|===\s*$");
re!(ADOC_LINK, r"(?:link:)?(https?://[^\s\[]+)\[([^\]]*)\]");
re!(ADOC_IMAGE, r"image::?([^\s\[]+)\[([^\]]*)\]");
re!(ADOC_ROLE, r"\[\.[A-Za-z0-9_-]+\]#([^#]*)#");
re!(ADOC_ADMONITION, r"^(NOTE|TIP|WARNING|IMPORTANT|CAUTION):\s+");

re!(TX_PRE_OPEN, r"^\s*<pre>(?:<code[^>]*>)?");
re!(TX_PRE_CLOSE, r"(?:</code>)?</pre>\s*$");
re!(TX_HEADING, r"^h[1-6](?:\([^)]*\)|\{[^}]*\}|\[[^\]]*\])*\.\s+");
re!(TX_BLOCK, r"^(?:bq|p|fn\d+)(?:\([^)]*\))?\.\s+");
re!(TX_LIST, r"^\s*[*#]+\s+(?:\[[ xX]\]\s+)?");
re!(TX_TABLE_HEAD, r"\|_\.\s*");
re!(TX_LINK, r#""([^"]+)":(\S+)"#);
re!(TX_IMAGE, r"!([^!\s]+)!");
re!(TX_MACRO, r"\{\{[^}]*\}\}");
re!(TX_INLINE, r"(^|[\s(])[*_\-+@]{1,2}|[*_\-+@]{1,2}([\s).,:;!?]|$)");
re!(HTML_TAG, r"</?[a-zA-Z][^>]*>");

re!(PLAIN_MARK, r"^\s*(?:[・•●○◆◇■□▼▽▶►\-*]|\(\d+\)|\d+\.|[①-⑳])\s*");
re!(PLAIN_BRACKET, r"^\s*【(.*)】\s*$");

fn strip_table_pipes(line: &str) -> String {
    line.trim().trim_matches('|').split('|').map(str::trim).collect::<Vec<_>>().join(" ")
}

impl Stripped {
    fn new(text: &str, format: Format) -> Self {
        let text = text.replace("\r\n", "\n");
        let mut out_lines = Vec::new();
        let mut in_code = false;
        let mut code_close = String::new();

        for raw in text.lines() {
            // --- コードブロック: 中身はそのまま、区切り行は除去 ---
            match format {
                Format::Md => {
                    if let Some(m) = MD_FENCE.captures(raw) {
                        let fence = m[1].to_string();
                        if !in_code {
                            in_code = true;
                            code_close = fence;
                            continue;
                        } else if raw.trim().starts_with(&code_close) {
                            in_code = false;
                            continue;
                        }
                    }
                }
                Format::Adoc => {
                    if ADOC_CODE_DELIM.is_match(raw) {
                        let d = raw.trim().to_string();
                        if !in_code {
                            in_code = true;
                            code_close = d;
                            continue;
                        } else if raw.trim() == code_close {
                            in_code = false;
                            continue;
                        }
                    }
                }
                Format::Textile => {
                    if !in_code && TX_PRE_OPEN.is_match(raw) {
                        let rest = TX_PRE_OPEN.replace(raw, "").to_string();
                        if TX_PRE_CLOSE.is_match(&rest) {
                            push(&mut out_lines, TX_PRE_CLOSE.replace(&rest, "").to_string());
                        } else {
                            in_code = true;
                            push(&mut out_lines, rest);
                        }
                        continue;
                    }
                    if in_code && TX_PRE_CLOSE.is_match(raw) {
                        in_code = false;
                        push(&mut out_lines, TX_PRE_CLOSE.replace(raw, "").to_string());
                        continue;
                    }
                }
                Format::Plain => {}
            }
            if in_code {
                push(&mut out_lines, raw.to_string());
                continue;
            }

            let line = match format {
                Format::Md => strip_md_line(raw),
                Format::Adoc => strip_adoc_line(raw),
                Format::Textile => strip_textile_line(raw),
                Format::Plain => strip_plain_line(raw),
            };
            push(&mut out_lines, line);
        }

        Stripped { text: out_lines.join("\n"), lines: out_lines }
    }
}

fn push(lines: &mut Vec<String>, line: String) {
    let t = line.trim();
    if !t.is_empty() {
        lines.push(t.to_string());
    }
}

fn strip_md_line(raw: &str) -> String {
    // 表の区切り行（|---|---|）と水平線は本文ではない
    if MD_TABLE_SEP.is_match(raw) && raw.contains('-') && raw.contains('|') {
        return String::new();
    }
    if MD_HR.is_match(raw) {
        return String::new();
    }
    let mut s = MD_QUOTE.replace(raw, "").to_string();
    if MD_HEADING.is_match(&s) {
        s = MD_HEADING.replace(&s, "").to_string();
    } else {
        s = MD_LIST.replace(&s, "").to_string();
    }
    if s.trim_start().starts_with('|') {
        s = strip_table_pipes(&s);
    }
    s = MD_IMAGE.replace_all(&s, "$1 $2").to_string();
    s = MD_LINK.replace_all(&s, "$1 $2").to_string();
    s = BACKTICK.replace_all(&s, "").to_string();
    s = MD_EMPH.replace_all(&s, "$1$2").to_string();
    HTML_TAG.replace_all(&s, "").to_string()
}

fn strip_adoc_line(raw: &str) -> String {
    if ADOC_DELIM.is_match(raw) || ADOC_ATTR_LINE.is_match(raw) || ADOC_TABLE_DELIM.is_match(raw) {
        return String::new();
    }
    let mut s = raw.to_string();
    if ADOC_HEADING.is_match(&s) {
        s = ADOC_HEADING.replace(&s, "").to_string();
    } else if s.trim_start().starts_with('.') && !s.trim_start().starts_with(". ") && !s.trim_start().starts_with("..")
    {
        // ブロックタイトル「.Title」
        s = s.trim_start()[1..].to_string();
    } else {
        s = ADOC_LIST.replace(&s, "").to_string();
    }
    s = ADOC_ADMONITION.replace(&s, "").to_string();
    if s.trim_start().starts_with('|') {
        s = strip_table_pipes(&s);
    }
    s = ADOC_IMAGE.replace_all(&s, "$2 $1").to_string();
    s = ADOC_LINK.replace_all(&s, "$2 $1").to_string();
    s = ADOC_ROLE.replace_all(&s, "$1").to_string();
    s = BACKTICK.replace_all(&s, "").to_string();
    s = s.replace("**", "").replace("__", "");
    s = MD_EMPH.replace_all(&s, "$1$2").to_string();
    s = s.trim_end_matches(" +").to_string();
    s
}

fn strip_textile_line(raw: &str) -> String {
    let t = raw.trim();
    if t == "---" || t == "***" {
        return String::new();
    }
    let mut s = raw.to_string();
    if TX_HEADING.is_match(&s) {
        s = TX_HEADING.replace(&s, "").to_string();
    } else if TX_BLOCK.is_match(&s) {
        s = TX_BLOCK.replace(&s, "").to_string();
    } else {
        s = TX_LIST.replace(&s, "").to_string();
    }
    s = MD_QUOTE.replace(&s, "").to_string();
    s = TX_TABLE_HEAD.replace_all(&s, "").to_string();
    if s.trim_start().starts_with('|') {
        s = strip_table_pipes(&s);
    }
    s = TX_MACRO.replace_all(&s, "").to_string();
    s = TX_IMAGE.replace_all(&s, "$1").to_string();
    s = TX_LINK.replace_all(&s, "$1 $2").to_string();
    s = HTML_TAG.replace_all(&s, "").to_string();
    s = TX_INLINE.replace_all(&s, "$1$2").to_string();
    s.replace("[[", "").replace("]]", "")
}

fn strip_plain_line(raw: &str) -> String {
    let t = raw.trim();
    if !t.is_empty() && t.chars().all(|c| matches!(c, '-' | '=' | '_' | '─' | '━' | '*')) {
        return String::new();
    }
    if let Some(c) = PLAIN_BRACKET.captures(raw) {
        return c[1].to_string();
    }
    let s = PLAIN_MARK.replace(raw, "").to_string();
    if s.contains(" | ") {
        return s.split(" | ").map(str::trim).collect::<Vec<_>>().join(" ");
    }
    s
}

// ---------------------------------------------------------------------------
// 正規化と bigram
// ---------------------------------------------------------------------------

/// NFKC・小文字化し、文字と数字だけを残す（空白・記号・句読点は比較対象外）。
fn normalize(s: &str) -> Vec<char> {
    s.nfkc().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

fn bigrams(chars: &[char]) -> HashMap<(char, char), usize> {
    let mut m = HashMap::new();
    if chars.len() == 1 {
        *m.entry((chars[0], '\0')).or_default() += 1;
    }
    for w in chars.windows(2) {
        *m.entry((w[0], w[1])).or_default() += 1;
    }
    m
}

/// 空白・タブ・表の区切りで分けた語ごとの bigram（語をまたぐ bigram は数えない）。
/// 表 → 箇条書き、行の組み替えなど、内容は同じで並びだけ変わる変換に強くする。
fn segments(line: &str) -> impl Iterator<Item = Vec<char>> + '_ {
    line.split(|c: char| c.is_whitespace() || matches!(c, '|' | '/' | '：' | ':'))
        .map(normalize)
        .filter(|n| !n.is_empty())
}

fn segment_bigrams(line: &str) -> HashMap<(char, char), usize> {
    let mut m = HashMap::new();
    for seg in segments(line) {
        for (k, v) in bigrams(&seg) {
            *m.entry(k).or_default() += v;
        }
    }
    m
}

fn line_bigrams(lines: &[String]) -> HashMap<(char, char), usize> {
    let mut m = HashMap::new();
    for l in lines {
        for (k, v) in segment_bigrams(l) {
            *m.entry(k).or_default() += v;
        }
    }
    m
}

fn intersection(a: &HashMap<(char, char), usize>, b: &HashMap<(char, char), usize>) -> usize {
    a.iter().map(|(k, v)| (*v).min(*b.get(k).unwrap_or(&0))).sum()
}

/// `lines` のうち、`other` にほぼ含まれない行を返す。
fn unmatched_lines(lines: &[String], other: &[char], other_bi: &HashMap<(char, char), usize>) -> Vec<String> {
    let other_str: String = other.iter().collect();
    let mut result = Vec::new();
    for line in lines {
        let n = normalize(line);
        if n.len() < 4 {
            continue;
        }
        let ns: String = n.iter().collect();
        if other_str.contains(&ns) {
            continue;
        }
        let bi = segment_bigrams(line);
        let total: usize = bi.values().sum();
        let hit: usize = bi.iter().filter(|(k, _)| other_bi.contains_key(k)).map(|(_, v)| *v).sum();
        if ratio(hit, total) < 0.7 {
            result.push(line.clone());
        }
    }
    result
}

// ---------------------------------------------------------------------------
// URL・数値
// ---------------------------------------------------------------------------

re!(URL, r"https?://[A-Za-z0-9\-._~:/?#@!$&*+,;=%]+");
re!(NUMBER, r"\d+(?:[.,:/\-]\d+)*");

fn urls(s: &str) -> Vec<String> {
    URL.find_iter(s)
        .map(|m| m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '。', '、']).to_string())
        .collect()
}

fn missing_urls(source: &str, output: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for u in urls(source) {
        if !output.contains(&u) && !seen.contains(&u) {
            seen.push(u);
        }
    }
    seen
}

fn number_counts(s: &str) -> HashMap<String, usize> {
    let s: String = s.nfkc().collect();
    // URL 内の数字は URL チェックで見るので除外
    let s = URL.replace_all(&s, " ");
    let mut m = HashMap::new();
    for n in NUMBER.find_iter(&s) {
        *m.entry(n.as_str().to_string()).or_default() += 1;
    }
    m
}

fn missing_numbers(source_stripped: &str, output_stripped: &str) -> Vec<String> {
    let src = number_counts(source_stripped);
    let out = number_counts(output_stripped);
    let mut v: Vec<String> =
        src.iter().filter(|(k, c)| out.get(*k).copied().unwrap_or(0) < **c).map(|(k, _)| k.clone()).collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// 構造比較
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
struct Counts {
    headings: usize,
    list_items: usize,
    table_cells: usize,
    code_blocks: usize,
    images: usize,
}

re!(MD_IMAGE_COUNT, r"!\[[^\]]*\]\([^)\s]+");
re!(ADOC_IMAGE_COUNT, r"image::?[^\s\[]+\[");
re!(TX_IMAGE_COUNT, r"!(?:https?://|/)[^!\s(]+(?:\([^)\n]*\))?!");
re!(TX_NUMBERED_PARA, r"^\d+\.\s+\S");
re!(TX_NESTED_ITEM, r"^[#*]{2,}\s");

re!(MD_TABLE_ROW, r"^\s*\|.*\|\s*$");
re!(TX_TABLE_ROW, r"^\s*\|.*\|\s*$");

fn count_structure(text: &str, format: Format) -> Counts {
    let mut c = Counts::default();
    let mut in_code = false;
    let mut in_table = false;
    let mut close = String::new();
    for line in text.lines() {
        match format {
            Format::Md => {
                if let Some(m) = MD_FENCE.captures(line) {
                    if !in_code {
                        in_code = true;
                        close = m[1].to_string();
                        c.code_blocks += 1;
                        continue;
                    } else if line.trim().starts_with(&close) {
                        in_code = false;
                        continue;
                    }
                }
                if in_code {
                    continue;
                }
                if MD_HEADING.is_match(line) {
                    c.headings += 1;
                } else if MD_LIST.is_match(line) && !MD_HR.is_match(line) {
                    c.list_items += 1;
                } else if MD_TABLE_ROW.is_match(line) && !MD_TABLE_SEP.is_match(line) {
                    c.table_cells += line.trim().trim_matches('|').split('|').count();
                }
            }
            Format::Adoc => {
                if ADOC_CODE_DELIM.is_match(line) {
                    if !in_code {
                        in_code = true;
                        close = line.trim().to_string();
                        c.code_blocks += 1;
                    } else if line.trim() == close {
                        in_code = false;
                    }
                    continue;
                }
                if in_code {
                    continue;
                }
                if ADOC_TABLE_DELIM.is_match(line) {
                    in_table = !in_table;
                    continue;
                }
                if in_table {
                    c.table_cells += line.matches('|').count();
                } else if ADOC_HEADING.is_match(line) {
                    c.headings += 1;
                } else if ADOC_LIST.is_match(line) {
                    c.list_items += 1;
                }
            }
            Format::Textile => {
                if !in_code && TX_PRE_OPEN.is_match(line) {
                    c.code_blocks += 1;
                    let rest = TX_PRE_OPEN.replace(line, "");
                    in_code = !TX_PRE_CLOSE.is_match(&rest);
                    continue;
                }
                if in_code {
                    if TX_PRE_CLOSE.is_match(line) {
                        in_code = false;
                    }
                    continue;
                }
                if TX_HEADING.is_match(line) {
                    c.headings += 1;
                } else if TX_LIST.is_match(line) || TX_NUMBERED_PARA.is_match(line) {
                    // 段落やコードを含む手順は「1. 手順」の段落で書く（Textile の規則）ので、項目として数える
                    c.list_items += 1;
                } else if TX_TABLE_ROW.is_match(line) {
                    c.table_cells += line.trim().trim_matches('|').split('|').count();
                }
            }
            Format::Plain => {}
        }
    }
    c.images = match format {
        Format::Md => MD_IMAGE_COUNT.find_iter(text).count(),
        Format::Adoc => ADOC_IMAGE_COUNT.find_iter(text).count(),
        Format::Textile => TX_IMAGE_COUNT.find_iter(text).count(),
        Format::Plain => 0,
    };
    c
}

fn compare_structure(source: &str, from: Format, output: &str, to: Format) -> Vec<StructureDiff> {
    if !from.is_structured() || !to.is_structured() {
        return Vec::new();
    }
    let s = count_structure(source, from);
    let o = count_structure(output, to);
    let mut diffs = Vec::new();
    if s.headings != o.headings {
        diffs.push(StructureDiff { kind: "headings", label: "見出し", source: s.headings, output: o.headings });
    }
    if s.list_items != o.list_items {
        diffs.push(StructureDiff {
            kind: "list_items",
            label: "リスト項目",
            source: s.list_items,
            output: o.list_items,
        });
    }
    if s.table_cells != o.table_cells {
        diffs.push(StructureDiff {
            kind: "table_cells",
            label: "表のセル",
            source: s.table_cells,
            output: o.table_cells,
        });
    }
    if s.code_blocks != o.code_blocks {
        diffs.push(StructureDiff {
            kind: "code_blocks",
            label: "コードブロック",
            source: s.code_blocks,
            output: o.code_blocks,
        });
    }
    if s.images != o.images {
        diffs.push(StructureDiff { kind: "images", label: "画像", source: s.images, output: o.images });
    }
    diffs
}

// ---------------------------------------------------------------------------
// 記法チェック
// ---------------------------------------------------------------------------

struct LintRule {
    re: Regex,
    message: &'static str,
}

fn lint_rules(to: Format) -> Vec<LintRule> {
    let r = |p: &str, message: &'static str| LintRule { re: Regex::new(p).unwrap(), message };
    let textile_heading = r(r"^h[1-6]\.\s", "Textile の見出し記法（h1. 等）が残っています");
    let adoc_heading = r(r"^={1,6}\s+\S", "AsciiDoc の見出し記法（= 等）が残っています");
    let adoc_table = r(r"^\|===", "AsciiDoc の表記法（|===）が残っています");
    let adoc_source = r(r"^\[source", "AsciiDoc のコード属性（[source]）が残っています");
    let pre = r(r"<pre>|</pre>", "Textile の <pre> が残っています");
    let md_heading = r(r"^#{1,6}\s+\S", "Markdown の見出し記法（#）が残っています");
    let md_link = r(r"\[[^\]\n]+\]\((?:https?://|/|\.)[^)]*\)", "Markdown のリンク記法が残っています");
    let md_table_sep = r(r"^\s*\|\s*:?-{3,}", "Markdown の表区切り行が残っています");
    let fence = r(r"^\s*```", "Markdown のコードフェンス（```）が残っています");
    let tx_link = r(r#""[^"\n]+":https?://"#, "Textile のリンク記法が残っています");
    let bq = r(r"^bq\.\s", "Textile の引用記法（bq.）が残っています");
    let tag = r(r"</?DOCSHIFT_INPUT", "入力の区切りタグが出力に残っています");
    let mut v = vec![tag];
    match to {
        Format::Md => v.extend([textile_heading, adoc_heading, adoc_table, adoc_source, pre, tx_link, bq]),
        Format::Adoc => v.extend([md_heading, md_link, md_table_sep, textile_heading, pre, tx_link, bq]),
        Format::Textile => v.extend([
            fence,
            md_link,
            md_table_sep,
            adoc_heading,
            adoc_table,
            adoc_source,
            r(
                r#""https?://[^"\s]+\.(?:svg|png|jpe?g|gif|webp)(?:\?[^"\s]*)?":"#,
                "画像が文字のリンクになっていて表示されません（!画像URL!:リンク先 と書きます）",
            ),
        ]),
        Format::Plain => v.extend([
            md_heading,
            md_link,
            md_table_sep,
            fence,
            textile_heading,
            adoc_heading,
            adoc_table,
            adoc_source,
            pre,
            tx_link,
            bq,
            r(r"\*\*[^*\n]+\*\*", "強調記法（**）が残っています"),
        ]),
    }
    v
}

fn lint(output: &str, to: Format) -> Vec<SyntaxIssue> {
    let rules = lint_rules(to);
    let mut issues = Vec::new();
    let mut in_code = false;
    let mut list_restart = TextileListRestart::default();
    for (i, line) in output.lines().enumerate() {
        // コードブロック内は対象外（Markdown のフェンス、AsciiDoc の ----、Textile の <pre>）
        let t = line.trim();
        let is_delim = match to {
            Format::Md => t.starts_with("```"),
            Format::Adoc => t == "----" || t == "....",
            Format::Textile => t.starts_with("<pre>") || t.ends_with("</pre>"),
            Format::Plain => false,
        };
        if is_delim && to != Format::Textile {
            in_code = !in_code;
            continue;
        }
        if to == Format::Textile {
            if t.starts_with("<pre>") {
                in_code = !t.ends_with("</pre>");
                list_restart.feed(i, "<pre>");
                continue;
            }
            if in_code {
                if t.ends_with("</pre>") {
                    in_code = false;
                }
                continue;
            }
        }
        if in_code {
            continue;
        }
        if to == Format::Textile
            && let Some(issue) = list_restart.feed(i, line)
        {
            issues.push(issue);
            continue;
        }
        for rule in &rules {
            if rule.re.is_match(line) {
                issues.push(SyntaxIssue {
                    line: i + 1,
                    text: line.chars().take(80).collect(),
                    message: rule.message.into(),
                });
                break;
            }
        }
    }
    issues
}

/// Textile の番号付きリストが段落やコードで途切れ、次の `#` で番号が 1 から振り直される箇所を見つける。
/// 見出しを挟んだ新しいリストは対象外（意図した振り直し）。
#[derive(Default)]
struct TextileListRestart {
    in_list: bool,
    /// 直前の番号付きリストが途切れてから、見出しが出ていない
    interrupted: bool,
}

impl TextileListRestart {
    fn feed(&mut self, index: usize, line: &str) -> Option<SyntaxIssue> {
        let t = line.trim_end();
        if t.starts_with("# ") || t == "#" {
            let restart = !self.in_list && self.interrupted;
            self.in_list = true;
            self.interrupted = false;
            return restart.then(|| SyntaxIssue {
                line: index + 1,
                text: line.chars().take(80).collect(),
                message: "番号付きリストが途中の段落やコードで切れ、番号が 1 から振り直されます".into(),
            });
        }
        if TX_HEADING.is_match(t) {
            self.in_list = false;
            self.interrupted = false;
        } else if TX_NESTED_ITEM.is_match(t) {
            // 入れ子の項目（## / #* / ** など）はリストの続き
        } else if self.in_list {
            // 空行・段落・コードなどでリストが終わる
            self.in_list = false;
            self.interrupted = true;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# 議事録\n\n## 参加者\n\n- 田中\n- 鈴木\n\n## 決定事項\n\n| 項目 | 期限 |\n| --- | --- |\n| 設計書 | 12/1 |\n\n詳細は [Wiki](https://example.com/wiki) を参照。\n\n```sh\nnpm run build\n```\n";
    const ADOC: &str = "= 議事録\n\n== 参加者\n\n* 田中\n* 鈴木\n\n== 決定事項\n\n[options=\"header\"]\n|===\n|項目 |期限\n\n|設計書 |12/1\n|===\n\n詳細は https://example.com/wiki[Wiki] を参照。\n\n[source,sh]\n----\nnpm run build\n----\n";
    const TEXTILE: &str = "h1. 議事録\n\nh2. 参加者\n\n* 田中\n* 鈴木\n\nh2. 決定事項\n\n|_. 項目 |_. 期限 |\n| 設計書 | 12/1 |\n\n詳細は \"Wiki\":https://example.com/wiki を参照。\n\n<pre><code class=\"sh\">\nnpm run build\n</code></pre>\n";

    #[test]
    fn faithful_conversions_pass() {
        for (out, to) in [(ADOC, Format::Adoc), (TEXTILE, Format::Textile)] {
            let r = check(MD, Format::Md, out, to);
            assert_eq!(r.grade, Grade::Ok, "{to}: {:#?}", r);
            assert!(r.recall > 0.97);
        }
    }

    #[test]
    fn detects_missing_content() {
        let broken = ADOC.replace("* 鈴木\n", "").replace("https://example.com/wiki[Wiki]", "Wiki");
        let r = check(MD, Format::Md, &broken, Format::Adoc);
        assert_ne!(r.grade, Grade::Ok);
        assert_eq!(r.missing_urls, vec!["https://example.com/wiki"]);
        assert!(r.structure.iter().any(|d| d.kind == "list_items"));
    }

    #[test]
    fn detects_foreign_syntax() {
        let r = check(MD, Format::Md, MD, Format::Textile);
        assert!(!r.syntax_issues.is_empty());
    }

    #[test]
    fn commentary_is_ng() {
        let out = format!("Note: I kept the title as the document title.\n\nWait, correction: here it is:\n\n{ADOC}");
        let r = check(MD, Format::Md, &out, Format::Adoc);
        assert_eq!(r.added_lines.len(), 2, "{:#?}", r.added_lines);
        assert_ne!(r.grade, Grade::Ok);
    }

    #[test]
    fn url_stops_at_fullwidth_punctuation() {
        assert_eq!(urls("（\"x\":https://a.example/issues/1）"), vec!["https://a.example/issues/1"]);
    }

    #[test]
    fn plain_table_to_markdown_table_is_ok() {
        let src = "宿題\n担当\t内容\t期限\n田中\t画面モック\t11/27\n";
        let out = "## 宿題\n\n| 担当 | 内容 | 期限 |\n| --- | --- | --- |\n| 田中 | 画面モック | 11/27 |\n";
        let r = check(src, Format::Plain, out, Format::Md);
        assert!(r.missing_lines.is_empty(), "{:#?}", r.missing_lines);
        assert_eq!(r.grade, Grade::Ok, "{:#?}", r);
    }

    #[test]
    fn headings_turned_into_bold_lines_are_not_ok() {
        let md = "# App\n\n[![CI](https://ci.example.com/b.svg)](https://ci.example.com/app)\n\nApp の説明\n\n## Setup\n\n本文です。\n\n### 認証\n\n1. トークンを取得します\n\n   説明です。\n\n2. ビルドします\n";
        let tx = "*App*\n\n\"https://ci.example.com/b.svg\":https://ci.example.com/app\n\nApp の説明\n\n*Setup*\n\n本文です。\n\n*認証*\n\n# トークンを取得します\n\n   説明です。\n\n# ビルドします\n";
        assert_eq!(crate::detect::detect(md), Format::Md);
        let r = check(md, Format::Md, tx, Format::Textile);
        assert_ne!(r.grade, Grade::Ok, "{:#?}", r);
    }

    #[test]
    fn textile_badge_as_text_link_and_list_restart_are_flagged() {
        let md = "# App\n\n[![CI](https://ci.example.com/b.svg?style=svg)](https://ci.example.com/app)\n\n1. 取得します\n\n   説明です。\n\n2. ビルドします\n";
        let tx = "h1. App\n\n\"https://ci.example.com/b.svg?style=svg\":https://ci.example.com/app\n\n# 取得します\n\n説明です。\n\n# ビルドします\n";
        let r = check(md, Format::Md, tx, Format::Textile);
        let msgs: Vec<_> = r.syntax_issues.iter().map(|i| (i.line, i.message.as_str())).collect();
        assert!(msgs.iter().any(|(_, m)| m.contains("画像が文字のリンク")), "{msgs:?}");
        assert!(msgs.iter().any(|(l, m)| *l == 9 && m.contains("振り直され")), "{msgs:?}");
        assert!(r.structure.iter().any(|d| d.kind == "images"), "{:#?}", r.structure);
        assert_ne!(r.grade, Grade::Ok);
    }

    #[test]
    fn textile_numbered_paragraph_steps_are_ok() {
        let md = "# App\n\n[![CI](https://ci.example.com/b.svg)](https://ci.example.com/app)\n\n1. 取得します\n\n   説明です。\n\n   ```sh\n   ls\n   ```\n\n2. ビルドします\n";
        let tx = "h1. App\n\n!https://ci.example.com/b.svg(CI badge)!:https://ci.example.com/app\n\n1. 取得します\n\n説明です。\n\n<pre><code class=\"sh\">\nls\n</code></pre>\n\n2. ビルドします\n";
        let r = check(md, Format::Md, tx, Format::Textile);
        assert_eq!(r.grade, Grade::Ok, "{:#?}", r);
    }

    #[test]
    fn textile_nested_and_consecutive_items_are_not_restarts() {
        let tx =
            "h2. 対策\n\n# 一つ目\n# 二つ目\n## 入れ子\n** 箇条書きの入れ子\n# 三つ目\n\nh2. 次\n\n# 新しいリスト\n";
        assert!(lint(tx, Format::Textile).is_empty(), "{:#?}", lint(tx, Format::Textile));
    }

    #[test]
    fn plain_output_markup_is_flagged() {
        let r = check(MD, Format::Md, MD, Format::Plain);
        assert!(!r.syntax_issues.is_empty());
    }
}
