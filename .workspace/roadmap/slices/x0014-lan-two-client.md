# x0014 — LAN 2-client presence（最小多人数）

> **ID:** `x0014` · **slug:** `lan-two-client` · **status:** done · **area:** `[host, client, zenoh, docs]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0013]`

目標: 同一 LAN で **2 クライアント**（PC 上 `alice` + ノートPC `bob`）が同じ room に入り、Room Token の **`sub` ごとに別 `user_root`**（entity 3 / 4）で移動・見えること。proto 変更なし。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [zenoh-router.md](../../0_doc/architecture/zenoh-router.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| zenohd | `tcp/0.0.0.0:7447` で LAN 待受。JWT ACL はしない |
| host | `verify` 後の `claims.sub` → Intent/Grab スロット（最大 2）。entity 3/4 |
| client | `AUTH_USER=alice\|bob` → 自 entity 3\|4。他 user_root は色付きプリミティブ |
| Identity | Room Token `sub` のみ（`InputEvent` に client_id なし） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | LAN: zenoh `0.0.0.0` + `ZENOH_CONNECT` + runbook | done |
| 2 | host per-sub Intent/Grab + user_root×2 | done |
| 3 | client: alice/bob→entity + 他プレイヤー描画 | done |
| 4 | smoke（同マシン2 client）+ README LAN 手順 | done |

## デモ規約（本 Slice）

| `AUTH_USER` | user_root entity | 接続順 |
|:---|:---|:---|
| `alice`（既定） | **3** | **先に接続**（host が最初の `sub` に slot0=entity 3 を割当） |
| `bob` | **4** | 2 人目 |

host は **初見 `sub` に空きスロットを first-come で割当**する。client の固定マップと一致させるため、**alice を先に起動**すること。3 人目は Intent drop（ログ）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Token `sub` ごとに Intent / Grab を分離（最大 2） | Steal / 3 人以上の動的スケール |
| host が entity 3/4 を frame に載せる（occupied のみ） | アバター GLB / 音声 / P2P |
| client は自 entity をカメラ追従、他は色付きボックス | 予測補間・ホスト移譲・verifier |
| `ZENOH_CONNECT` + BIND_ALL で LAN 到達可能 | VPS リレー・公開範囲 ACL |
| 各 client が別ボックスを掴める | 既掴み entity の奪取 |

## やらない（後続）

- 汎用 UUID→entity 発見プロトコル（3 人以上）
- Steal / 投げ / アバター GLB
- `ROOM_ID` → Zenoh キー本配線
- 予測補間・launcher・verifier

## 起動（同一マシン smoke）

```bash
# PC: auth/world/assets + zenohd + host
zenohd -c zenoh-router/config.json5
cd client && cargo run -p host-server

# Terminal A
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice cargo run -p client

# Terminal B（alice のあと）
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
```

## 起動（LAN・ノートPC）

PC（WSL）で zenohd / Docker（`BIND_ALL=true`）/ host / alice client。ノートPC:

```bash
export PC_IP=<PCのLAN_IP>
export AUTH_URL=http://$PC_IP:4002
export WORLD_URL=http://$PC_IP:4003
export ASSETS_URL=http://$PC_IP:4004
export ZENOH_CONNECT=tcp/$PC_IP:7447
ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=bob cargo run -p client
```

Windows ホスト側ファイアウォールで **7447** と **4002–4004** を許可すること。

## 実装メモ

- `ZENOH_CONNECT`（既定 `tcp/127.0.0.1:7447`）— host / client
- host: per-sub `PoseOverride` + `PhysicsWorld` user_root×2 + per-slot Grab（他 slot 保持中は無視）
- client: `local_user_root_entity` + 他 root を box mesh で描画
- smoke: `bash .scripts/smoke-lan-two-client.sh`

← [目次](../README.md)
