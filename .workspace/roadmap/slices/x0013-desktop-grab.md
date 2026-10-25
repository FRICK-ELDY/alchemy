# x0013 — Desktop Grab（最小）

> **ID:** `x0013` · **slug:** `desktop-grab` · **status:** done · **area:** `[client]` · **surface:** `[desktop]` · **depends_on:** `[x0011, x0012]`

目標: Desktop・ダッシュ閉・look capture 中に **RMB** でカメラ前方レイが当たった Grabbable ボックスを掴み、host 既存の kinematic 追従（x0011）へ載せる。離したら放下。ワイヤ schema は変えず、aim は **user_root（眼）位置**。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| `world_hit` | Desktop / XR 共通の AABB レイヒット（surface 非依存） |
| surface Desktop | look capture 中の LMB / RMB / E → ヒット開始／保持／解放。aim = eye。床箱は俯瞰が必要 |
| `MoveIntent` | 既存 `GRAB` + `grab_entity_id` + `aim_*`（XR と同契約） |
| host | 変更なし（x0011 の Grab hold をそのまま使う） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `world_hit` 抽出（AABB / grabbable 共通） | done |
| 2 | Desktop: look 前方レイ + RMB Grab hold | done |
| 3 | smoke + client.md / README | done |

## Desktop バインド（本 Slice 追加）

| 入力 | 役割 |
|:---|:---|
| LMB（look 開始後・再押し長押し） | 画面中央レイヒットの箱を Grab。capture 直後の1クリックでは掴まない |
| RMB / E（look capture 中） | 同上 Grab |
| Esc（ダッシュ開） | Grab もクリア（XR と同様） |

箱は床上。水平視線では当たらないので **下を向いて箱が明るくなる**まで合わせてから掴む。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| look capture 中・ダッシュ閉時、RMB で entity 10..18 を掴める | ダッシュ開中・look 未 capture での Grab |
| 保持中は眼位置＋開始時 offset で host kinematic 追従（既存） | 回転追従・投げ・クロスヘア／ハイライト必須 |
| 離す／Esc でダッシュ開 → Grab 解放 | Steal / 2 ハンド / 感度 UI |
| system_ui は Grab を知らない | WorldRoot・Facet 並べ替え |
| proto / PROTOCOL_PIN 変更なし | host 側ロジック変更 |

## やらない（後続）

- XR 回転追従 / release impulse（投げ）
- 画面中央クロスヘア・ヒットハイライト
- Steal / Receivable / Scalable
- 感度・バインドの Settings Facet

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
```

確認:

1. ダッシュ閉でクリック → look capture  
2. マウスで **下を向き**、箱が明るくなったら LMB/RMB/E 長押し → 箱が眼位置に追従（WASD で動かすと分かりやすい）  
3. 離すと箱が落ちる。Esc でダッシュを開くと Grab も解除  

## 実装メモ

- `client/app/src/world_hit.rs` — Ray / AABB / grabbable ヒット（XR laser からも利用）
- `client/app/src/surface/desktop.rs` — RMB + look 前方レイ → Intent Grab
- host / proto: 変更なし（x0011）
- smoke: `bash .scripts/smoke-desktop-noclip.sh` · `bash .scripts/smoke-rapier-host.sh`

← [目次](../README.md)
