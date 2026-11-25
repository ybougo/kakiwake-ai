# SampleApp

[![CI](https://ci.example.com/badge.svg)](https://ci.example.com/sample-app)
[![code style](https://img.example.com/style.svg)](https://style.example.com/)

SampleApp のバックエンドと認証機能

## Setup

基本的には DevContainer を利用して開発してください。
VS Code で本リポジトリを開き、`Dev Containers: Reopen in Container` を実行します。

### バックエンド

```sh
# DB の起動
$ docker compose up -d db cache
# セットアップ
$ bin/setup
```

### 認証機能

[AuthLib](https://auth.example.com/) を使った認証用アプリケーションです。プライベートなパッケージを使うため、トークンを設定します。

1. アクセストークンを取得します

   下記を参考に、トークンを取得します。

   [トークンの作成手順](https://docs.example.com/tokens)

   取得したトークンを環境変数 `PKG_TOKEN` として設定します。

   ```sh
   export PKG_TOKEN="<トークン>"
   ```

2. パッケージをインストールします

   ```sh
   $ npm install
   ```

3. ビルドします

   ```sh
   $ npm run build
   ```

   これで `public/css/app.css` が作成されます。

## seed データの選択

- `dev`: 開発用。何も指定しない場合はこのデータが投入されます。
- `minimum`: 最小限のデータです。
  - 新規の契約会社を作成するときに使います。
- `prod`: 本番環境用です。

| 環境変数       | 説明                 | 初期値 |
| -------------- | -------------------- | ------ |
| `SEED_PROJECT` | 投入するデータの種類 | `dev`  |
| `SEED_ENV`     | 投入する環境         | `dev`  |

http://localhost:9001/ にアクセスすると管理画面が表示されます。  
ユーザー名は `user` です。
