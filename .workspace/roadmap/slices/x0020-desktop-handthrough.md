# x0020 — Desktop client 手通し

> **ID:** `x0020` · **slug:** `desktop-handthrough` · **status:** `done` · **area:** `[client, host, auth, world, docs, ci]` · **depends_on:** `[x001f]`

目標: 手元で **Desktop client** が  
`auth login → Room Token → host-server → Zenoh → 描画開始`  
まで手通しできることを、**日常スタックの正本手順 + 機械 smoke** で保証する。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md)（P3 品質基盤の延長）・x001f 後続

## 設計方針

1. **正本スタックは日常開発**（auth/world/assets `:4002/:4003/:4004` + zenohd `:7447`）
2. Hermetic e2e（`:4102` 系 / Zenoh `:7547`）は env 併記のみ（Desktop CI 必須化しない）
3. smoke は **host ログ + client build/units** のみ。Desktop client / e2e-probe / GUI は起動しない（login→Zenoh→描画は手動チェックリスト）。未起動サービスは **exit 1**
4. 手動手通し（host 常駐）と smoke（一時 host）は **排他**。同一 `ROOM_ID` の既存 host は止めてから smoke
5. Room Token 自動更新（10 分デモ）は **含めない** → 後続 P1-1

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | README 手通し手順 + `client/env.desktop.example` | done |
| 2 | `smoke-desktop-handthrough.sh` + Makefile | done |
| 3 | 成功条件確認・後続リンク | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| README の Desktop 手順で host + client が日常ポートに繋がる | Tunnel / WAN / R2 / OpenXR |
| silent login 後、host が Manifest/physics を出す（smoke が機械確認） | client login→Zenoh→描画の機械 assert・GUI・e2e-probe（手動のみ） |
| e2e ポートへ向ける env 例が文書にある | e2e 上での Desktop CI 必須化 |
| サービス未起動時は smoke が赤 | Room Token 自動更新・JWKS bg・host role・magnitude clamp |

## やらない（後続）

- P1 デモゲート（→ [x0021](x0021-p1-demo-gate.md)）
- SceneManifest 駆動（P4 / 後続）
- GUI E2E（クリック自動化）
- e2e compose を Desktop の既定ポートに変更すること

## 成功条件

1. 文書どおり日常スタックで Desktop client 手通しができる（下記チェックリスト）
2. 日常スタック起動済み・**他 host 停止済み**なら smoke が緑  
   （assert 範囲は host Token/Manifest/physics + client build/units。client 描画経路は含めない）
3. スタック未起動なら同 smoke が赤
4. Token TTL / 10 分セッションは成功条件に含めない

### 手動確認チェックリスト

1. auth / world / assets を各 `docker compose up -d`、`zenohd -c zenoh-router/config.json5`
2. `cd client && cargo run -p host-server` → ログに Room Token verified / SceneManifest / physics
3. 別ターミナル: `ALCHEMY_FORCE_DESKTOP=1 AUTH_USER=alice AUTH_PASSWORD=Password1 cargo run -p client` → ウィンドウが開き描画される  
   （UI login: `AUTH_USER` を省略し Home タブから login。release は password または `ALCHEMY_AUTH_TOKEN` 必須）
4. （任意）e2e スタック向けは [`client/env.desktop.example`](../../../client/env.desktop.example) のコメント例を export
5. 機械検証する場合は **手順 2 の host を止めてから** `make smoke-desktop-handthrough`（3a と 3b は排他）

## 起動メモ

```bash
# 日常（正本）— 3a 手動 / 3b smoke は排他
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5

# 3a
cd client
cargo run -p host-server
AUTH_USER=alice AUTH_PASSWORD=Password1 ALCHEMY_FORCE_DESKTOP=1 cargo run -p client

# 3b（host 停止後）
make smoke-desktop-handthrough
```

環境変数の一覧・e2e 併記: [`client/env.desktop.example`](../../../client/env.desktop.example)

← [目次](../README.md)
