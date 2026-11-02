# x006a 元仕様 — world-server への Zenohex 導入・安全ラッパー

> **正本スライス:** [slices/x006a-world-zenohex-adapter.md](slices/x006a-world-zenohex-adapter.md)  
> **位置づけ:** 実装着手時の入力仕様（会話で提示された全文）。採択後の読み替え・保証境界はスライス側が正。  
> **読み替え:** 現行 `world-server` は umbrella ではない。`apps/network` / `Network.Zenoh.*` → **`World.Network.Zenoh.*`**（単一アプリ `:world`）。

---

## 1. 背景とアーキテクチャ上の位置づけ

Alchemy では、かつて `world-server` 内に存在した空間・物理演算（Rapier3d / Formula VM）の NIF を完全廃止し、100% Pure Rust 製の `host-server` へと分離した。
これにより、`world-server`（Elixir）は毎秒 20〜60Hz の重たい空間座標中継のホットパスから外れ、以下の**コントロールプレーンおよび監査・指揮ロール**に専念する設計となっている。

1. **ルーム制御・空間台帳**（`apps/core` / `apps/network` ※当時想定）: Room Token 発行、Scene Manifest の署名配信、エンティティ制御指令。
2. **空間オーディオの指揮者**（`apps/contents` ※当時想定）: 非同期オーディオコマンドの発行。
3. **ホスト改ざん検証・死活監視**（`apps/verifier` 等 ※当時想定）: `host-server` が配信するフレームの抜き打ち監査（Spot-Check）、`client_info` の監視、Zenoh Liveliness による死活監視とフェイルオーバー昇格指令。

これらを実現するため、Elixir から Zenoh ネットワーク（`zenohd`）へ接続する手段として `zenohex`（Rustler ベースのプリコンパイル済み NIF ライブラリ）を導入する。

---

## 2. 厳守すべき3つの安全設計ルール（NIF リスクの封じ込め）

`zenohex` は内部で Rustler NIF を使用する。BEAM VM の安定性と CI の再現性を100%維持するため、以下の3つのガードレールを必須要件とする。

### ルール①：`:ring` チャネル設定の強制（デッドロック防止）

- `zenohex` のデフォルト設定（`channel_kind: :fifo`, `capacity: 256`）では、Elixir プロセスのメールボックス処理が一瞬でも詰まると Rust 側の Zenoh コールバックスレッドがブロック（停止）し、通信全体がデッドロックする。
- これを防ぐため、`config/config.exs` で必ず **`config :zenohex, channel_kind: :ring, channel_capacity: 1024`** を指定すること。これにより、バースト受信時も古い未処理サンプルがドロップされるだけで、VM やスレッドは絶対に停止しない。

### ルール②：Behaviour（Adapter パターン）による直接依存の隔離

- ビジネスロジック（`core` / `contents` / `verifier` 等）から `Zenohex.*` モジュールを直接呼び出すことを禁止する。
- 必ず Adapter（Behaviour）と Client ラッパーを経由させること。
- これにより、将来「Zenohex だけを別 BEAM ノード（分散 Erlang）へ隔離する」または「Rust Port プロセスへ差し替える」という判断になった際も、アプリケーション側のコードを1行も変更せず移行できる構造を担保する。

**採択後のモジュール名:** `World.Network.Zenoh.Adapter` / `Client` / `ZenohexAdapter` / `LocalStubAdapter`。

### ルール③：CI / `mix test` 環境でのスタブ分離と `credo --strict` 準拠

- `./.scripts/ci.sh world`（Docker 内での `format / compile / credo / mix test`）実行時、外部に `zenohd` が起動していなくても全テストがパスするよう、`config/test.exs` では NIF セッションを開かない `LocalStubAdapter` に切り替えること。
- 既存の `mix credo --strict`（0 issues）およびフォーマット規約を完全に満たすこと。

---

## 3. 実装ステップ（Phase 1 〜 Phase 3）

### Phase 1: 依存関係の追加と設定ファイルの整備

1. `world-server` の対象アプリ（当時想定: `apps/network/mix.exs` → **採択: ルート `mix.exs`**）に `{:zenohex, "~> 0.10.1"}` を追加する。
   - ※プリコンパイル済み NIF が自動ダウンロードされるため、特別な理由がない限りローカル Rust ビルド強制フラグは立てないこと（ただし Docker CI 環境のターゲットアーキテクチャでプリコンパイルバイナリが取得できるか確認すること）。
2. `config/config.exs` に以下を追加する：

   ```elixir
   config :zenohex,
     channel_kind: :ring,
     channel_capacity: 1024
   ```

3. `config/runtime.exs` で環境変数 `ZENOH_CONNECT`（例: `tcp/127.0.0.1:7447` や `tls/relay.example.tld:7447`）を読み込み、接続先エンドポイントを設定できるようにする。未指定時はローカルの `tcp/127.0.0.1:7447` を既定値とする。

### Phase 2: ラッパー＆アダプタ群の実装

`World.Network.Zenoh`（当時: `Network.Zenoh`）内に以下を実装する。

1. **Adapter** (Behaviour)
   - `@callback publish(key_expr :: String.t(), payload :: binary()) :: :ok | {:error, term()}`
   - `@callback subscribe(key_expr :: String.t(), subscriber_pid :: pid()) :: {:ok, term()} | {:error, term()}`
   - `@callback unsubscribe(sub_id :: term()) :: :ok | {:error, term()}`
2. **Client** — 設定された Adapter へ処理を委譲する統一インターフェース。
3. **ZenohexAdapter** (GenServer)
   - Application Supervisor のツリーに組み込む（`:test` では `LocalStubAdapter`）。
   - 起動時に `Zenohex.Nif.Logger.enable()` を呼び、`:ring` バッファのドロップ警告を検知できるようにする。
   - `Zenohex.Session.open/1` でセッションを確立し、接続失敗時は Supervisor ごとクラッシュさせず 3 秒間隔のリトライ（`Process.send_after`）。
   - `publish/2` 時は `key_expr` ごとに Publisher ID を Map キャッシュする。
4. **LocalStubAdapter** — `mix test` 用インメモリアダプタ（NIF を介さず Publish / Subscribe の単体テストを可能にする）。

**採択後の追加契約（レビュー反映）:**

- 購読配信の正本は `{:zenoh_sample, key_expr, payload}`（`Adapter.sample_message/2`）。`%Zenohex.Sample{}` は `ZenohexAdapter` 内で正規化し業務コードへ漏らさない。
- 本スライスの接続保証は **open 失敗のリトライまで**。セッション存続の能動監視・reconnect 後の自動 re-subscribe は非保証。

### Phase 3: トピック規則の適合と単体テスト・CI 検証

1. キー式は現行の `wire_cap` トピック体系（`alchemy/...`）をそのまま扱えるようにし、特定のトピック名を Adapter 内にハードコードしないこと。
2. `LocalStubAdapter` を用いた `Client` の単体テストを追加する（配信契約の正規化テストを含む）。
3. 以下がエラーや警告ゼロであること：
   - `mix format --check-formatted`
   - `mix credo --strict`
   - `mix test`
   - （可能であれば `./.scripts/ci.sh world`）

---

## 4. 本スライスの非目標（後続）

- Verifier Spot-Check / Liveliness 監視 / 空間オーディオ指揮など、業務ロジックからの実トピック配線
- world→zenohd の WAN mTLS クライアント身元
- umbrella 分割（`apps/network` 新設）

← [スライス x006a](slices/x006a-world-zenohex-adapter.md) · [目次](README.md)
