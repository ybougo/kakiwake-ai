# docshift 仕様 v1.0

## 1. 目的

4 つの形式の間で、見出し・リスト・表・強調・コード・引用・リンクの体裁を保ったまま相互変換する。変換はすべて LLM CLI の単発実行で行う。

## 2. 決定事項

| 項目           | 決定                                                                                                                                                            |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 対象 OS        | Windows ネイティブ（Tauri 2 / WebView2）                                                                                                                        |
| 形式           | `plain` / `md` / `adoc` / `textile`（12 方向）                                                                                                                  |
| Teams 出力     | 廃止。Markdown の文字列は Teams で書式にならず、リッチテキスト（HTML）は編集しにくいため                                                                        |
| 変換方式       | すべて LLM。ルールベースの変換は使わない                                                                                                                        |
| プロバイダ     | Claude Code / Codex CLI / Copilot CLI の 3 つすべてに対応                                                                                                       |
| 既定モデル     | Claude `claude-sonnet-5-5`、Codex `gpt-6.1-terra`、Copilot `mai-code-2`                                                                                         |
| プレビュー     | なし                                                                                                                                                            |
| 画面           | 入力・変換先・結果・コピー・品質バッジのみ。詳細情報は出さない                                                                                                  |
| 進捗表示       | 生成中の文字を順に表示（Claude: stream-json の text_delta、Copilot: stdout をそのまま）。Codex は途中経過を出さないため完了時に表示。変換中は進捗バーとスピナー |
| 推論量の既定   | Claude `low`、Codex `none`（品質同等、速度もほぼ同じ）、Copilot `low`（`none` は Textile の見出し・表を変換し損ねた）                                           |
| フロントエンド | Svelte 5 + TypeScript + Vite                                                                                                                                    |
| 外部通信       | LLM 以外への送信なし（アプリは CSP で外部接続を禁止。CLI のテレメトリ・自動更新・MCP・Web 検索は停止）                                                          |
| 品質チェック   | 変換ごとにローカルで実行。結果を OK / WARN / NG で表示。NG のときの自動再変換は設定で選ぶ                                                                       |
| 認証           | 各 CLI のログインをそのまま使う。起動時に状態を確認し、未ログインならログイン用コンソールを開くボタンを出す                                                     |

Copilot の既定は当初 MAI Code 2 Flash だったが、AsciiDoc のバッジを Markdown のまま残す、コードブロックを壊す、推論量 `none` で Textile の見出し・表を変換し損ねる、といった失敗が Flash にだけあったため MAI Code 2 に変更した（1 件あたり約 2 秒遅い）。

Claude の既定は当初 Haiku 5.5 だったが、評価で 33 件中 3 件、元にない行の追加があった。Sonnet 5.5 は 11 件中 0 件で、所要時間の差は約 1 秒だったため Sonnet に変更した。

## 3. 変換の流れ

1. 変換元が「自動判定」なら `detect::detect` で形式を推定する
2. `prompt::build` でプロンプトを組み立てる（規則、変換元と変換先の記法早見表、変換先ごとの追加指示、区切りタグで囲んだ本文）
3. `provider::run` で CLI を起動する
   - 空の一時ディレクトリをカレントにし、終了後に削除する
   - 引数は改行を含まない短いものだけにする（Windows の `.cmd` シム対策）。規則と本文は stdin で渡す
   - タイムアウトとキャンセルに対応。ウィンドウは出さない（`CREATE_NO_WINDOW`）
4. `postprocess::clean` で、前置き文・区切りタグの残り・全体を囲むコードフェンスを除去する
5. `quality::check` で品質を判定する

生成途中のテキストは `provider::run` の `on_text` で受け取り、Tauri の `Channel` で画面へ送る。最終結果は後処理済みのテキストで置き換える。

## 4. CLI の呼び出し

| CLI     | 主な引数                                                                                                                                                                                                                                |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| claude  | `-p --model M --tools "" --strict-mcp-config --no-session-persistence --setting-sources "" --output-format stream-json --verbose --include-partial-messages --system-prompt <1行> [--effort E]`                                         |
| codex   | `exec -m M --sandbox read-only --skip-git-repo-check --ephemeral --color never -c mcp_servers={} -c web_search="disabled" -c analytics.enabled=false -c check_for_update_on_startup=false [-c model_reasoning_effort=E] -o <file> -`    |
| copilot | `-s --model M --no-auto-update --no-custom-instructions --available-tools= [--disable-builtin-mcps --no-ask-user --disallow-temp-dir --disable-client-telemetry] [--reasoning-effort E]`（`-p` を付けず、stdin 全体をプロンプトにする） |

Copilot の角かっこ内のフラグは版によって存在しないため、実際に動く版の `--help` を見て、対応しているものだけを付ける。

## 5. 品質チェック

| 指標                  | 内容                                                                               |
| --------------------- | ---------------------------------------------------------------------------------- |
| recall                | 記法を外した本文の語単位 bigram のうち、出力にも含まれる割合                       |
| precision             | 出力の bigram のうち、元に含まれる割合                                             |
| missing / added lines | 行単位で、相手側にほぼ含まれない行                                                 |
| URL / 数値            | 元の URL・数値が出力に残っているか                                                 |
| 構造                  | 見出し・リスト項目・表のセル・コードブロックの数（plain が関わる場合は比較しない） |
| 記法の混入            | 変換先にない記法が残っていないか                                                   |

- **NG**: 出力が空、recall < 0.85、precision が基準 − 0.15 未満、URL の欠落が 3 件以上
- **WARN**: いずれかの指摘あり
- **OK**: 指摘なし

## 6. 設定

`%APPDATA%\dev.docshift.app\settings.json` に保存する。

- 選択中のプロバイダ、プロバイダごとの設定（パス・モデル・推論量・追加引数）、追加したモデル ID
- タイムアウト（10〜1800 秒、既定 180 秒）、品質チェック（既定オン）、NG のとき再変換（既定オフ）
- 変換先ごとの追加指示、最後に使った変換元と変換先
