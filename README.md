# Kakiwake AI

プレーンテキスト・Markdown・AsciiDoc・Redmine（Textile）の 4 形式を、体裁を保ったまま相互変換する Windows 向けデスクトップアプリです。
変換は手元にインストールした LLM CLI（Claude Code / Codex CLI / GitHub Copilot CLI）の単発実行で行います。

```
[Claude Code] [Codex CLI] [Copilot CLI]  [Sonnet 5.5 ▾]                           ⚙
┌ [自動判定 ▾] ──────────────┐         ┌ [Text][Markdown][AsciiDoc][Textile]  OK [コピー] ┐
│  入力                      │ 変換 →  │  結果（生成中の文字から順に表示）                │
└────────────────────────────┘         └──────────────────────────────────────────────────┘
```

## 対応形式

| ID        | 形式                    | 出力の方針                                                        |
| --------- | ----------------------- | ----------------------------------------------------------------- |
| `plain`   | プレーンテキスト        | 記法を外し、`【見出し】` `■` `・` `\|` 区切りで見た目の構造を残す |
| `md`      | Markdown（GFM）         | 表・タスクリスト・コードフェンス対応                              |
| `adoc`    | AsciiDoc（Asciidoctor） | `=` 見出し、`\|===` 表、`[source]` + `----`                       |
| `textile` | Redmine Textile         | `h1.`、`\|_.` 表、`<pre><code class="…">`、`{{toc}}` 維持         |

変換元は「自動判定」も選べます。

## 必要なもの

- Windows 10 / 11（WebView2。インストーラーに同梱）
- 変換に使う CLI のどれか（1 つ以上）
  - [Claude Code](https://docs.claude.com/claude-code) `claude`
  - [Codex CLI](https://github.com/openai/codex) `codex`
  - [GitHub Copilot CLI](https://github.com/github/copilot-cli) `copilot`
- ビルドする場合: Node.js 24 以上、Rust 1.90 以上（Windows 上でビルドするなら Visual Studio Build Tools（C++）も。WSL からのクロスビルドなら不要）

## 認証

Kakiwake AI 自身は認証情報を持ちません。各 CLI のログイン状態をそのまま使います。

| CLI         | ログイン            | API キー／トークンで使う場合        | 状態確認             |
| ----------- | ------------------- | ----------------------------------- | -------------------- |
| Claude Code | `claude auth login` | `ANTHROPIC_API_KEY`                 | `claude auth status` |
| Codex CLI   | `codex login`       | `OPENAI_API_KEY`                    | `codex login status` |
| Copilot CLI | `copilot login`     | `COPILOT_GITHUB_TOKEN` / `GH_TOKEN` | （なし）             |

- 起動時に各 CLI の有無とログイン状態を確認し、プロバイダ名の横の点（緑 = 利用可、赤 = 利用不可）で表示します。
- 未ログインのときは画面下にエラーと **［ログイン］** ボタンが出ます。押すと新しいコンソールでログインコマンドが開きます（ブラウザまたはデバイスコードによる認証は CLI 側の機能を使います）。
- Copilot CLI には状態確認コマンドがないため、未ログインの場合は変換を実行した時点でエラーになります。

## 使い方

1. 上部でプロバイダとモデルを選ぶ
2. 左に貼り付ける（変換元は通常「自動判定」のままで可）
3. 右上で変換先を選び、**変換**（`Ctrl+Enter`）。中止は `Esc`
4. **コピー** して貼り付け先へ

- 結果は生成中の文字から順に表示されます（Claude・Copilot）。Codex は CLI が途中経過を出さないため、完了時にまとめて表示されます。変換中は結果欄の上端に進捗バーが流れます。
- 結果の横のバッジは品質チェックの結果です（`OK` / `WARN` / `NG`）。マウスを乗せると理由が表示されます。

### モデルの切り替え

- モデルのドロップダウンには、プロバイダごとのおすすめモデルが並びます。最後に選んだモデルはプロバイダごとに記憶します。
- 新しいモデルが出たときは **「＋ モデル ID を追加…」** を選び、モデル ID を入力して Enter を押します。候補に追加され、そのまま選択されます。追加したモデルは `×` か設定画面から削除できます。

| プロバイダ  | 既定                | 候補                                                                                           |
| ----------- | ------------------- | ---------------------------------------------------------------------------------------------- |
| Claude Code | `claude-sonnet-5-5` | `claude-haiku-5-5`（最速）、`claude-opus-5-5`                                                  |
| Codex CLI   | `gpt-6.1-terra`     | `gpt-6.2-sol`（高品質）、`gpt-6.1-sol`                                                         |
| Copilot CLI | `mai-code-2`        | `mai-code-2-flash`（約 2 秒速いが、AsciiDoc のバッジやコードブロックを崩すことがある）、`auto` |

### 設定（⚙）

- **プロバイダ**: 実行ファイルのパス（空欄なら PATH から検出）、モデル ID、推論量、追加引数
  - 推論量の既定: Claude `low`、Codex `none`、Copilot `low`（Copilot は `none` だと Textile の見出しや表を変換し損ねることがある）
- **変換・品質**: 品質チェックの有効・無効、NG のとき 1 回だけ自動で再変換する、タイムアウト
- **追加指示**: 変換先ごとにプロンプトへ追加する指示（例:「見出しの番号は削除する」）

設定は `%APPDATA%\dev.kakiwake-ai.app\settings.json` に保存されます。

## 品質チェック

変換後に、ローカルで次の点を検査します（LLM は使いません）。

- **内容の保持**: 記法を外して正規化した本文の bigram で、再現率（元の内容がどれだけ残ったか）と適合率（元にない内容が混ざっていないか）を測る
- **欠落と追加**: 元の行・URL・数値が出力に残っているか、元にない行（注釈・説明文など）が混ざっていないか
- **構造**: 見出し・リスト項目・表のセル・コードブロックの数
- **記法の混入**: 変換先にない記法（例: Textile 出力に残った Markdown の表区切り）

`NG` の条件は、出力が空、内容の保持が 85% 未満、元にない内容が多い、URL が 3 件以上欠落のいずれかです。

## 外部通信とセキュリティ

- 入力文書は、選んだ LLM CLI を通じてその LLM にだけ送られます。Kakiwake AI 自身は外部と通信しません（CSP で外部接続を禁止し、フォント・画像もすべてローカル。Tauri のプラグインも使っていません）。
- CLI は、空の一時ディレクトリをカレントにし、本文を stdin で渡して起動します。変換にツールは不要なので、各 CLI のツールを止めています。

| CLI     | ツールの止め方                                                                                                                              | 確認結果                                                                                                                                          |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude  | `--tools ""`、`--strict-mcp-config`、`--setting-sources ""`                                                                                 | ツールなし。ファイルを読む指示にも応じない                                                                                                        |
| Copilot | `--available-tools=none`、`--deny-tool=shell/write/read/memory/url`、`--disable-builtin-mcps`、`--no-ask-user`                              | ツールなし。`--available-tools=`（空）は「制限なし」扱いで bash が動いたため、この指定にしている                                                  |
| Codex   | `--sandbox read-only`、`features.*=false`（シェル、画像読み込み、ChatGPT 連携アプリ、Web 検索、プラグイン、サブエージェントなど）、MCP 無効 | シェル・外部連携は使えない。**ファイル編集（apply_patch）だけは止める設定がなく、read-only でも作業ディレクトリ外に書けた**（codex 0.162 で確認） |

- **Codex の残るリスク**: 文書に紛れた指示に従ってファイルを書き換える可能性があります。対策として Codex のイベントを監視し、ファイル変更やコマンド実行が現れた時点でプロセスを止め、対象のパスをエラーとログに出します（書き込み自体は防げません）。出どころの分からない文書の変換には、Claude か Copilot を使ってください。
- テレメトリ・自動更新は止めています（Claude: `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` ほか、Codex: `analytics.enabled=false`・更新確認なし、Copilot: `COPILOT_AUTO_UPDATE=false`・クライアントテレメトリ無効）。CLI 本体の認証・LLM 呼び出しに伴う通信は各 CLI の仕様に従います。
- ログ（`%LOCALAPPDATA%\dev.kakiwake-ai.app\logs\`）には文書の本文を書きません。

## 開発

### Windows 上でビルド

```powershell
npm ci
npm run tauri dev      # 開発起動
npm run tauri build    # インストーラー作成（target\release\bundle\ 以下に msi / nsis）
```

### WSL / Linux からクロスビルド

```bash
scripts/build-windows.sh                                   # target/x86_64-pc-windows-msvc/release/kakiwake-ai.exe
scripts/build-windows.sh --copy /mnt/c/Users/<you>/Apps/kakiwake-ai   # ビルド後に Windows 側へコピー
```

sudo 不要です。初回だけ `llvm-rc`（apt のパッケージを展開するだけ）と、xwin で Microsoft CRT / Windows SDK を `~/.cache/kakiwake-xbuild` に取得します（Microsoft のライセンスへの同意が前提）。インストーラーは作らず、exe 単体を出力します。

| コマンド                                                                 | 内容                                                                                                                              |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `npm run lint`                                                           | Prettier / ESLint / svelte-check（警告もエラー扱い）/ rustfmt / clippy（`-D warnings`）                                           |
| `npm run format`                                                         | Prettier と rustfmt で整形                                                                                                        |
| `npm test`                                                               | コアロジックの単体テスト                                                                                                          |
| `npm run eval -- [--providers claude,codex] [--model codex=gpt-6.2-sol]` | 実際の CLI でサンプル文書を変換し、品質チェックの結果と所要時間（最初の文字が出るまでの時間を含む）を `eval-out/report.md` に出力 |

### 構成

```
crates/kakiwake-core/   GUI に依存しないコア（WSL / Linux でもテスト可能）
  src/format.rs         形式定義と、プロンプト用の記法早見表
  src/prompt.rs         プロンプト生成
  src/provider.rs       CLI の起動（引数・環境変数・タイムアウト・キャンセル・認証確認）
  src/postprocess.rs    前置き文・全体を囲むコードフェンスの除去
  src/detect.rs         変換元形式の自動判定
  src/quality.rs        品質チェック
  src/bin/kakiwake-eval.rs  評価ツール
  tests/fixtures/       評価用サンプル文書
src-tauri/              Tauri 2 アプリ（コマンド・設定保存）
src/                    Svelte 5 + TypeScript の画面
docs/SPEC.md            仕様
```

## 既知の制約

- 変換は LLM の出力です。品質チェックで多くの崩れは検出できますが、完全ではありません。重要な文書は目視で確認してください。
- 1 回の変換に数秒〜十数秒かかります。大半は CLI の起動と 1 往復の固定コストです（実測の目安: Claude 約 3 秒、Codex・Copilot 約 10 秒）。
- exe は署名していません。Windows のスマート アプリ コントロールが有効な環境では起動がブロックされます。配布する場合はコード署名が必要です。

## ライセンス

[MIT](LICENSE)
