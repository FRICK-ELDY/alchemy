# x006a — world-server へ Zenohex 導入（Adapter 隔離）

> **ID:** `x006a` · **slug:** `world-zenohex-adapter` · **status:** `done` · **area:** `[world, zenoh, ci]` · **depends_on:** `[x004e, x0066]`

目標: `world-server`（Elixir コントロールプレーン）から Zenoh 網へ接続する手段として **`zenohex`（Rustler プリコンパイル NIF）** を導入し、**`:ring` チャネル強制**と **Behaviour / Adapter 隔離**で BEAM 安定性と差し替え可能性を担保する。業務ロジック（Room / Verifier / contents）からの `Zenohex.*` 直接参照は禁止。

入力仕様（全文・読み替え注記付き）: [../x006a-zenohex-intro-spec.md](../x006a-zenohex-intro-spec.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `World.Network.Zenoh.Client` 経由の publish / subscribe のみが業務向け入口 | 業務トピック配線（Spot-Check / liveliness 監視 / オーディオ指揮）— 後続スライス |
| 購読配信形は Adapter 共通の `{:zenoh_sample, key_expr, payload}`。`%Zenohex.Sample{}` は `ZenohexAdapter` 内で正規化して閉じる | key expression スキーマの変更；`wire_cap` 秘匿 |
| `config :zenohex, channel_kind: :ring, channel_capacity: 1024`（コールバックデッドロック防止） | `:fifo` 互換；容量チューニングの最適値 |
| test は `LocalStubAdapter`（zenohd / NIF セッション不要）で `mix test` / world CI が通る | Docker 外ホストでの musl 以外ターゲット検証 |
| open 失敗時は Supervisor を落とさず 3s 間隔で再接続試行；publisher は `key_expr` キャッシュ | **セッション存続の能動監視**；reconnect 後の **自動 re-subscribe**；WAN mTLS クライアント身元（world→zenohd） |
| `ZENOH_CONNECT`（未指定時 `tcp/127.0.0.1:7447`） | エンドポイント発見 / Connect Override UI |

## 完了条件

1. `{:zenohex, "~> 0.10.1"}` が world に入り、Alpine CI でプリコンパイル NIF（musl）が取得できる  
2. `mix format --check-formatted` / `mix credo --strict`（0 issues）/ `mix test` / `./.scripts/ci.sh world`（相当）が緑（zenohd なし）  
3. `LocalStubAdapter` と `ZenohexAdapter.normalize_sample/1` で配信契約がテスト固定されている  
4. 未知の `:zenoh_adapter` は起動時に fail-closed  

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | deps + `:ring` + `ZENOH_CONNECT` + test stub 切替 | done |
| 2 | Adapter / Client / LocalStub / ZenohexAdapter（正規化・monitor・open リトライ） | done |
| 3 | 単体テスト + credo / world CI | done |
| 4 | PR マージ・status → done | done |

## 実装メモ

- 単一 Mix アプリ `:world` のためモジュールは `World.Network.Zenoh.*`（umbrella `apps/network` は作らない）
- `Zenohex.*` 呼び出しは `ZenohexAdapter` のみ。NIF 受信は forwarder 経由で契約タプルへ変換
- GenServer.call 既定 timeout 5s（制御プレーン想定の直列化；過負荷方針は後続）
- Hex 依存追加は本スライスで人承認済み前提

← [目次](../README.md)
