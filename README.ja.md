# SSH for Gemini

[English README (README.md)](./README.md)

**Gemini Task モード**から自宅や手元の Linux マシンへ本物の OpenSSH で接続するためのツールセットです。**Mac 不要・グローバルへの TCP 22 番公開不要・WebSocket 不要・HTTP 側の追加認証不要**で動きます。

依存ゼロの極小 Rust デーモン（strip 済み約 600 KB）が純粋な HTTP/1.1 の `POST`/`GET` をローカルの `127.0.0.1:22` に中継し、同梱の Gemini Skill 雛形（`skill-template/`）と組み合わせることで、携帯やブラウザの Gemini Task から自分の Linux PC を直接ぶっ叩いてコードレビュー・ビルド・テスト・ハードウェア確認・バックグラウンドジョブ投入などを行えるようになります。

---

## できること

- **携帯の Gemini アプリから自宅 Linux を操作**: 外出先からでも Gemini Task モードで自宅 Linux のシェルを叩き、リポジトリの調査や実作業を任せられます。
- **Gemini Task クォータの有効活用**: Antigravity や Claude Code などのローカル開発エージェントとは別枠の Gemini クォータで、コードレビューや雑務・ビルド確認を回せます。
- **認証・暗号化は 100% OpenSSH + Ed25519 任せ**: トランスポートは Cloudflare Tunnel 越しの純粋 HTTPS ですが、裏は `127.0.0.1:22` の `sshd` に直結しているため、HTTP 側で自前認証や Cloudflare Access のメール認証を別途組む必要がありません。
- **長時間タスクの非同期実行**: Linux 側で `tmux` や `systemd-run --user` にジョブを逃がして即リターンさせることで、Gemini 側のターン時間制限を跨いで作業を継続できます。

---

## 技術選定の経緯と Gemini Task 実行環境の制約

Google 公式のデスクトップ自動操作・ローカルタスク連携は Apple Silicon Mac 先行で、別デバイスからのリモート起動も同一 Wi-Fi / Bluetooth が前提となっています。一方、Gemini Task モード自体にはアカウント共有のクラウド実行環境（リモートコンピュータ）があり、設定で **「リモート コンピュータのネットワーク アクセス」** をオンにすると外向き通信が可能になります。

このリモート環境を実測した結果、以下の 5 つの制約が判明し、それがそのまま `ssh-for-gemini` の設計理由になっています。

| Gemini Task リモート環境で判明した制約 | 一般的な手法が詰まる理由 | `ssh-for-gemini` の解決策 |
| :--- | :--- | :--- |
| **外向き TCP 22 番が塞がれている** | 素の `ssh` はタイムアウトする。 | すべて 443 番 HTTPS に乗せる。 |
| **L7 の TLS インスペクション付きゲートウェイが `Upgrade: websocket` を弾く（`400 Bad Request`）** | `cloudflared access ssh`（WebSocket）や Tailscale DERP（`Upgrade: derp`）がハンドシェイクで落ちる。 | WebSocket を使わず、通常の HTTP/1.1 `POST`（`/tx`）とロングポーリング `GET`（`/rx`）に `Content-Length` と Keep-Alive を付けて TCP バイト列を中継する。 |
| **`/dev/net/tun` が無く非特権コンテナ** | カーネルモードの WireGuard / Tailscale が使えない。 | Python 3 標準ライブラリのみで動くユーザー空間 `ProxyCommand`（`http-ssh-proxy.py`）を採用。 |
| **サンドボックスポリシーで `~/.ssh` への書き込みと `ssh-keygen` がブロックされる** | Gemini 側で鍵生成や `~/.ssh` 配置を行うとポリシー拒否になる。 | Linux PC 側で専用 Ed25519 鍵を生成し、Gemini 側では `$HOME/.local/share/<鍵名>`（`chmod 600`）に置く。 |
| **スキルの自動セキュリティスキャナが、パッケージ内の生秘密鍵（および露骨な Drive 持ち出し記述）を弾く** | 秘密鍵を `SKILL.md` や `assets/` に同梱するとスキル保存が `ZIP_NOT_CLEAN` で失敗する。 | **二段構え**にする：**L1**＝アカウント共有のリモート環境 `$HOME/.local/share/...`（別タスクでも共有される）、**L2**＝自分専用の Google Drive フォルダ（例：`gemini/my-linux-ssh/`）。環境がリセットされたときだけ Drive から自動復元する簡潔な記述をスキルに書く。 |

その他の実行環境特性：
- **User-Agent**: Gemini のリモート環境からの外向き HTTP リクエストには `Google-Agent` の User-Agent ヘッダが必要です（`skill-template/scripts/http-ssh-proxy.py` に組み込み済み）。
- **Google Drive 読み戻し時のエスケープ**: Drive 上にテキストとして保存され Google ドキュメント扱いになった鍵を読み出すと、先頭の `-----` が `\-----` にエスケープされたり段落間に空行が入ったりします。スキル雛形では復元時に先頭バックスラッシュと空行を正規化してから `chmod 600` する手順を明記しています。
- **ターン・時間制限**: Gemini Task の 1 ターンは無限ではないため、重いビルドやテストは Linux 側で `tmux` / `nohup` / `systemd-run --user` にデタッチして即リターンさせ、後続ターンでログやペインを見に行く運用を前提としています。

---

## リポジトリ構成

```text
ssh-for-gemini/
├── README.md                          # 英語 README（冒頭に日本語版へのリンクあり）
├── README.ja.md                       # 日本語 README（このファイル）
├── LICENSE-MIT / LICENSE-APACHE       # デュアルライセンス (MIT OR Apache-2.0)
├── Cargo.toml / Cargo.lock            # 外部依存ゼロの Rust クレート
├── src/main.rs                        # HTTP/1.1 POST/GET <-> ローカル TCP(22) デーモン（既定ポート 28787）
├── flake.nix / flake.lock             # Nix flake（packages, checks, devShells, overlays, nixosModules）
├── package.nix                        # rustPlatform.buildRustPackage 定義
├── nix/module.nix                     # ハードニング済み NixOS systemd モジュール (services.ssh-for-gemini)
└── skill-template/                    # Gemini Task にスキル登録させるための雛形
    ├── SKILL.md
    └── scripts/http-ssh-proxy.py
```

---

## 必要な環境

1. **Linux マシン**: Ed25519 公開鍵認証を有効にした `sshd` が動いていること。
   - **強く推奨**: 普段使いのユーザーではなく、専用の非特権「檻」ユーザー（例：`llm`）を用意し、触ってよいディレクトリやグループだけを割り当ててください。
2. **Cloudflare Tunnel**: 任意ドメインのサブドメイン（例：`https://ssh-bridge.example.com`）を **`http://127.0.0.1:28787`**（サービス種別は SSH ではなく **HTTP**）へ転送する設定。
   - NixOS をお使いなら、宣言的にリモート管理できる Cloudflare Tunnel 構成として [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely) もあわせてどうぞ。
3. **Gemini アカウント**: Gemini Task モードが使え、**「設定」→「Task 設定」→「リモート コンピュータのネットワーク アクセス」** をオンにできること。

---

## コーディングエージェント（Antigravity / Claude Code 等）向け：セットアップ誘導ガイド

ユーザーがこのリポジトリをローカルのコーディングエージェント（Antigravity、Claude Code 等）に読み込ませてセットアップを依頼した場合、エージェントは以下の役割分担で進めてください。

### Phase 1 — コーディングエージェントが Linux PC 側でやること
1. **ユーザーに 4 項目を確認する**:
   - Gemini 専用の Linux ユーザー名（未作成なら非特権ユーザー `llm` 等の作成を提案）
   - Cloudflare Tunnel で生やす公開サブドメイン（例：`ssh-bridge.example.com`）
   - ローカル待受ポート（既定：`28787`）
   - Gemini スキル名（例：`my-linux-ssh`）と Google Drive 上のバックアップ先フォルダ名（例：`gemini/my-linux-ssh`）
2. **`ssh-for-gemini` をビルド・常駐化する**:
   - NixOS：`nix build` で動作確認するか、`nixosModules.default`＋`overlays.default` を NixOS 構成に組み込み `services.ssh-for-gemini.enable = true;` を有効化する。
   - その他 Linux：`cargo build --release` でビルドし、systemd サービス化する。
3. **Gemini 専用の Ed25519 鍵ペアを生成する**:
   ```bash
   ssh-keygen -t ed25519 -N "" -C "gemini-task-ssh" -f ./id_ed25519_gemini
   ```
   - `id_ed25519_gemini.pub` を専用ユーザーの `~/.ssh/authorized_keys` に登録する（ポート転送等が不要なら `no-port-forwarding,no-agent-forwarding,no-X11-forwarding` 等のオプション付与を推奨）。
4. **Cloudflare Tunnel を設定する**:
   - `https://<サブドメイン>` の転送先を **`http://127.0.0.1:28787`**（タイプ：**HTTP**）に設定する。
   - NixOS 環境なら [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely) を参考にしてもよい。
5. **疎通確認を行う**:
   - `curl -fsS http://127.0.0.1:28787/healthz` が `ok` を返すこと。
   - `curl -fsS https://<サブドメイン>/healthz` が `ok` を返すこと。
6. **`skill-template/` をユーザー環境向けに書き換えたコピーを用意する**:
   - `my-linux-ssh`, `my-linux-box`, `ssh-bridge.example.com`, `llm`, `my_linux_key`, `gemini/my-linux-ssh` を実環境に合わせて置換する。
   - **重要**: 秘密鍵の中身は `SKILL.md` や `skill-template/` に絶対に含めないこと（セキュリティスキャナに弾かれます）。

### Phase 2 — ユーザーへの引き継ぎ（Gemini Task 側への橋渡し）
Phase 1 が通ったら、コーディングエージェントはユーザーへ以下の 3 ステップを案内してください。

1. **Gemini のネットワーク設定をオンにする**: Gemini の **設定 → Task 設定 → リモート コンピュータのネットワーク アクセス** を有効化する。
2. **専用鍵を Google Drive に置く**: 生成した `id_ed25519_gemini`（必要なら `.pub` も）を自分の非公開 Google Drive フォルダ `gemini/<スキル名>/` に配置する（最初の Gemini Task チャットに一度貼って Drive に保存させる運用でも可）。
3. **Gemini Task にスキルを作成させる**: カスタマイズ済みの `SKILL.md` と `scripts/http-ssh-proxy.py` を **Gemini Task** のチャットに添付（または貼り付け）し、以下のように指示する：

   > 添付した `SKILL.md` と `scripts/http-ssh-proxy.py` を使って、`<スキル名>` というマルチファイルスキルを作成してください（秘密鍵はスキルに含めず、Google Drive の `gemini/<スキル名>/` に置いてあります）。スキル保存後、`<サブドメイン>` へ SSH 接続して `uname -a && id` を実行テストしてください。

---

## 手動セットアップ手順

### 1. デーモンのビルドと起動（Linux PC 側）

Nix を使う場合：
```bash
nix build
./result/bin/ssh-for-gemini --listen 127.0.0.1 --port 28787 --target-port 22
```

テスト実行：
```bash
nix flake check
```

Nix を使わない場合（Rust 1.70+）：
```bash
cargo test
cargo build --release
./target/release/ssh-for-gemini --listen 127.0.0.1 --port 28787 --target-port 22
```

### 2. NixOS モジュールでの常駐（任意）

NixOS の flake inputs に本リポジトリを追加すると、ハードニング済みの systemd サービスを宣言的に有効化できます。

```nix
{
  services.ssh-for-gemini = {
    enable = true;
    listen = "127.0.0.1";
    port = 28787;
    targetHost = "127.0.0.1";
    targetPort = 22;
    maxSessions = 16;
    idleTimeout = 180;
  };
}
```

### 3. Cloudflare Tunnel ルート設定

Cloudflare Tunnel で公開サブドメイン（例：`ssh-bridge.example.com`）の転送先を **HTTP** の `http://127.0.0.1:28787` に向けます。

```bash
curl -fsS https://ssh-bridge.example.com/healthz
# -> ok
```

NixOS での Cloudflare Tunnel 管理には [kenh0u/nixos-cloudflared-remotely](https://github.com/kenh0u/nixos-cloudflared-remotely) も活用できます。

### 4. プロトコル仕様

- `GET /healthz` → `200 OK`（`ok\n`）
- `POST /open` → `target_host:target_port` へ TCP 接続を開き、32 文字の 16 進セッション ID を返却（`200 OK`）
- `POST /tx/<sid>` → リクエストボディの生バイト列を TCP ソケットへ書き込み（`204 No Content`）
- `GET /rx/<sid>` → TCP ソケットをロングポーリング（既定 15 秒、5ms パケット結合あり）し、生バイト列（`200 OK`）・タイムアウト（`204 No Content`）・EOF（`410 Gone`）を返却
- `POST /close/<sid>` → セッションを閉じて解放（`204 No Content`）

---

## セキュリティ上の注意

- **専用の「檻」ユーザーを使う**: 普段使いのアカウントやパスワード無し `sudo` 権限を与えず、自律エージェントに触らせても良い範囲だけに閉じた非特権ユーザーを用意してください。
- **用途専用の鍵ペアにする**: Gemini Task 専用の Ed25519 鍵を発行しておけば、不要になったときに `authorized_keys` から 1 行消すだけで即座に無効化できます。
- **スキル本体に秘密鍵を入れない**: 生の秘密鍵をスキルに同梱せず、`$HOME/.local/share/...`（L1）＋自分専用 Google Drive フォルダ（L2）の構成を使ってください。
- **必ず `127.0.0.1` にバインドする**: `ssh-for-gemini` を `0.0.0.0` に晒さず、ループバック限定＋Cloudflare Tunnel 経由で運用してください。

---

## ライセンス

以下のいずれかのライセンスの下で提供されます（Rust エコシステム標準のデュアルライセンス）：

- Apache License, Version 2.0（[LICENSE-APACHE](./LICENSE-APACHE) または http://www.apache.org/licenses/LICENSE-2.0）
- MIT License（[LICENSE-MIT](./LICENSE-MIT) または http://opensource.org/licenses/MIT）
