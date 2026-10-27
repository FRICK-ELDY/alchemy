# x0024 — 配送クラス契約（D1 + D3）

> **ID:** `x0024` · **slug:** `delivery-class` · **status:** `done` · **area:** `[protocol, client, docs]` · **depends_on:** `[x0023]`

目標: 負債マップ D1（アプリ向け配送契約の欠落）と D3（欠落デルタ時の再同期未契約）を **保証として固定**する。(1) 現行 Zenoh キーの配送クラス対応表。(2) `frame` 欠落時は再送せず `snapshot` 取り直し。

根拠: 負債マップ（D1/D3・配送クラス固定）／kokosync 系の配送語彙を Alchemy キーに写した契約。詳細メモは作業者ローカルの private ノート参照可。

正本: [protocol/peer-delivery.md](../../../protocol/peer-delivery.md) · [protocol.md](../../0_doc/architecture/protocol.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| D1 | protocol / docs | `protocol/peer-delivery.md` に latest / request-response / best-effort / Blob の対応表。`not-sent` / `unknown` 語彙。確実配送 ACK 層は作らない |
| D3 | protocol / host / client | `FRAME_TICK_GAP_RESYNC`（既定 1＝**publish 連番**）。host は空デルタで tick を進めない。欠落なら適用せず `…/snapshot`。queryable は x0023 のまま |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `peer-delivery.md` + architecture リンク | done |
| 2 | `decide_frame_tick` + client gap→snapshot + 単体 | done |
| 3 | architecture 保証表更新 | done |

## 契約

### 配送クラス（D1）

詳細は [peer-delivery.md](../../../protocol/peer-delivery.md)。要約:

| 経路 | クラス |
|:---|:---|
| `…/frame` | latest（デルタ）。全 tick 到達は保証しない |
| `…/snapshot` | request/response フル状態 |
| `…/manifest` / `…/presence` | latest（低頻度） |
| `…/input` | best-effort Intent（`intent_seq` は host 受理順のみ） |
| `…/host_control` | best-effort（既定 OFF） |
| assets HTTP CAS | Blob（参照）。Zenoh Blob ではない |

- **受信成功**: デコード可能なバイトを受け付けたことまで。アプリ適用完了は別契約
- **`not-sent` / `unknown`**: 確実配送が無い現状でも語彙だけ固定。`unknown` を別 ID で自動再送しない
- **欠落デルタ**: 再送しない → ローカルは unknown → snapshot

### Frame 欠落 → snapshot（D3）

| 条件 | 動作 |
|:---|:---|
| host: 空デルタ | publish しない / **`net_tick` も進めない** / snapshot tick = **最後に publish した tick** |
| client: `tick <= last` | 捨てる（x0023） |
| client: `last < tick <= last + FRAME_TICK_GAP_RESYNC` | デルタ適用（既定 gap=1 → 連番のみ） |
| client: `tick > last + FRAME_TICK_GAP_RESYNC` | **デルタ未適用** → `fetch_snapshot` 置換（クールダウン 500ms） |
| snapshot 経路 | tick ゲートなし（x0023 据え置き） |

- `PROTOCOL_PIN`: **据え置き**（`alchemy-protocol/0.9.0-x0023`）。ワイヤスキーマ不変

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 現行キーの配送クラスが `peer-delivery.md` で読める | 編集・インベントリ用確実配送チャネル / event ID ACK |
| tick 欠落時に中途半端なデルタを entity 表へ載せない | 欠落フレームの再送・補間・jitter buffer |
| 欠落検知後に snapshot で表を置換収束できる | snapshot 連続 query のレート保証・診断メトリクス |
| `FRAME_TICK_GAP_RESYNC` が protocol クレートの契約値 | Manifest / Presence 署名（D4） |

## やらない（後続）

- 確実配送（sendTo 相当）と event ID 重複排除の実装
- Manifest / Presence の world 署名（D4）
- RenderFrame quat / scale（E1）
- 経路別送受信・RTT 診断（表は未計測 = null）

## 成功条件

1. `protocol/peer-delivery.md` に全現行キーのクラスと語彙がある  
2. `decide_frame_tick(last=10, tick=12)` → resync；`(10, 11)` → apply（単体緑）  
3. client が gap 時にデルタ未適用で snapshot を取る  
4. late join（Manifest 後 snapshot）が壊れない  
5. `PROTOCOL_PIN` 不変。`cargo test -p alchemy-protocol -p client` 緑  

## 起動メモ

日常スタックは x0023 のまま（PIN 据え置き）。

```bash
zenohd -c zenoh-router/config.json5
# host-server → client（Desktop）

cd client && cargo test -p alchemy-protocol -p client
```

← [目次](../README.md)
