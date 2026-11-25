use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// 変換対象の文書形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Plain,
    Md,
    Adoc,
    Textile,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Plain, Format::Md, Format::Adoc, Format::Textile];

    pub fn id(self) -> &'static str {
        match self {
            Format::Plain => "plain",
            Format::Md => "md",
            Format::Adoc => "adoc",
            Format::Textile => "textile",
        }
    }

    /// プロンプトに埋め込む形式名。
    pub fn prompt_name(self) -> &'static str {
        match self {
            Format::Plain => "plain text (no markup language)",
            Format::Md => "Markdown (GitHub Flavored Markdown)",
            Format::Adoc => "AsciiDoc",
            Format::Textile => "Redmine Textile",
        }
    }

    /// その形式の記法の早見表（プロンプト用）。
    pub fn syntax_guide(self) -> &'static str {
        match self {
            Format::Plain => PLAIN_GUIDE,
            Format::Md => MD_GUIDE,
            Format::Adoc => ADOC_GUIDE,
            Format::Textile => TEXTILE_GUIDE,
        }
    }

    /// 構造（見出し・表など）を記法として持つ形式か。
    pub fn is_structured(self) -> bool {
        !matches!(self, Format::Plain)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Format {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "plain" | "txt" | "text" => Ok(Format::Plain),
            "md" | "markdown" => Ok(Format::Md),
            "adoc" | "asciidoc" => Ok(Format::Adoc),
            "textile" | "redmine" => Ok(Format::Textile),
            other => Err(format!("unknown format: {other}")),
        }
    }
}

const PLAIN_GUIDE: &str = "Plain text has no markup. Structure is shown only by layout and simple symbols.
When READING plain text, infer structure from layout:
- A short line followed by body text, a line wrapped in 【】, or a line starting with ■ □ ● ◆ ▼ or \"1.\" \"1-1.\" \"第1章\" is usually a heading.
- Lines starting with ・ - * • ○ or \"(1)\" \"①\" \"1.\" \"a)\" are list items; deeper indentation means nesting.
- Rows whose cells are separated by tabs, \"|\", or aligned with runs of spaces form a table; the first row is usually the header.
- Lines indented by 4+ spaces or that look like commands/source code are code.
When WRITING plain text, use exactly this style:
- Heading level 1: 【Heading】 on its own line. Level 2: \"■ Heading\". Level 3 and deeper: \"□ Heading\". Put a blank line before each heading.
- Bullet list: \"・item\"; each nesting level adds 2 leading spaces.
- Numbered list: \"1. item\", \"2. item\"; nesting adds 2 leading spaces.
- Table: one row per line, cells separated by \" | \", header row first, no separator line.
- Code block: every line indented by 4 spaces. Inline code: keep the text as-is.
- Emphasis (bold/italic/strike): remove the markers, keep the words.
- Link: \"text (URL)\"; if the text equals the URL write the URL only.
- Quote: indent the quoted lines by 2 spaces and keep the words.
- Horizontal rule: a line of \"----------\".
- No other symbols or markup.";

const MD_GUIDE: &str = "GitHub Flavored Markdown:
- Headings: \"# H1\", \"## H2\" ... \"###### H6\".
- Bold **text**, italic *text*, strikethrough ~~text~~, inline code `code`.
- Bullet list \"- item\" (nest with 2 spaces); numbered list \"1. item\" (nest with 3 spaces); task \"- [ ] todo\" / \"- [x] done\".
- Code block: ``` on its own line, optional language after the opening fence, closing ```.
- Quote: \"> text\". Horizontal rule: \"---\" on its own line with blank lines around.
- Link [text](URL); bare URLs are fine. Image ![alt](URL).
- Table: \"| a | b |\" header row, then \"| --- | --- |\", then body rows.
- Line break inside a paragraph: end the line with two spaces or use a blank line for a new paragraph.";

const ADOC_GUIDE: &str = "AsciiDoc (Asciidoctor):
- Document title \"= Title\" (only for the top heading); sections \"== H2\", \"=== H3\", \"==== H4\".
  Map Markdown \"#\"→\"=\", \"##\"→\"==\", \"###\"→\"===\" (one \"=\" per level).
- Bold *text*, italic _text_, monospace `code`, strikethrough [.line-through]#text#.
- Bullet list \"* item\", nested \"** item\", \"*** item\". Numbered list \". item\", nested \".. item\".
  Checklist \"* [ ] todo\" / \"* [x] done\".
- Code block:
  [source,lang]
  ----
  code
  ----
  (omit \",lang\" when unknown: just \"----\" delimiters).
- Quote block: \"____\" delimiters, or \"> text\" for a single line is NOT AsciiDoc — use the ____ block.
- Horizontal rule: \"'''\".
- Link: URL[text] or link:URL[text]; bare URL is fine. Image: image::URL[alt].
- Table:
  [options=\"header\"]
  |===
  |Head A |Head B

  |cell 1 |cell 2
  |===
- A blank line separates blocks; put a blank line before and after lists, tables and code blocks.";

const TEXTILE_GUIDE: &str = "Redmine Textile:
- Headings: \"h1. Title\", \"h2. Section\", \"h3. Sub\" — at the start of a line, followed by a blank line.
- Bold *text*, italic _text_, strikethrough -text-, underline +text+, inline code @code@.
- Bullet list \"* item\", nested \"** item\". Numbered list \"# item\", nested \"## item\". Mixed nesting \"#*\".
  There is no task list syntax: write \"* [ ] todo\" / \"* [x] done\" literally.
- Code block: <pre>...</pre>; with a language: <pre><code class=\"ruby\">...</code></pre>.
- Quote: \"bq. text\" for one paragraph, or lines starting with \"> \".
- Link: \"text\":URL ; bare URL is fine. Image: !URL!.
- Table: header row \"|_. Head A |_. Head B |\", body rows \"| cell 1 | cell 2 |\".
- Horizontal rule: \"---\" on its own line with blank lines around.
- Table of contents macro: {{toc}}. Keep Redmine macros like {{toc}} and wiki links [[Page]] as-is when the target is Textile; otherwise drop {{toc}} and keep [[Page]] as plain text \"Page\".";
