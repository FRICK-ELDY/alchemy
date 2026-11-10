# x007b — scene despawn

> **ID:** `x007b` · **slug:** `scene-despawn` · **status:** `done` · **area:** `[world, host, client, protocol, relay, docs]` · **depends_on:** `[x006d, x006e, x0069, x0039]`  
> **優先度:** backlog P0-次

目標: host role の Desktop Inventory から **scene mesh（64-hex blake3 付き entry）** を部屋から取り除く。経路は take-out（x006d）対称の **host 向け低頻度制御面**（Zenoh `…/inventory/despawn`）。world が scene から削除・revision bump し、host は Rapier + shared scene + reseal を適用する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P0-次** · x006e（put-back は部屋を残す）が後回しにした world DELETE / host despawn / reseal

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| world: host Room Token で `DELETE …/scene/entries/:entity_id`（mesh blake3 必須・revision bump）。**DB 楽観 CAS は [x0082](x0082-scene-revision-cas.md)** | user_root / cuboid（blake3 無し）削除；guest DELETE |
| Zenoh `…/inventory/despawn` は Room AEAD（`AeadKind::InventoryDespawn`）。host role 検証後に world HTTP | Intent 経路；guest despawn |
| world 削除成功後は **shared scene + 再 seal を必ず適用**。Rapier 除去はその後最良努力（失敗時も scene は権威）。host 内は `revision` 単調（新しい shared のみ採用；stale spawn/despawn は skip） | 分散 TX（world↔host↔guest）；Rapier 失敗時の自動再同期；HTTP 完了順の保証 |
| Desktop Inventory: scene mesh 選択 → **Despawn**（host）。put-back とは独立 | XR Inventory；Prefab 多子；put-back と一体 TX |
| WAN ACL は spawn と同型で `inventory/despawn` を通す（app が role 検証） | ACL だけで host を保証 |

## 設計（要点）

```
Client UI ─Room AEAD (InventoryDespawn)─► Zenoh …/inventory/despawn
despawn worker ─open+host role─► world DELETE …/scene/entries/:entity_id
physics tick ─► shared scene / force_reseal → Rapier remove（HTTP なし）
```

- 削除キー = `entity_id`（entry に 64-hex `blake3` があるものだけ）
- 平文 Room Token を Zenoh に載せない（x0069）
- world HTTP は despawn worker（物理 60Hz 外）
- put-back（コピー保存）とは独立；Despawn は部屋から消すだけ（Drive は触らない）
- host 内 `shared_scene` は revision 単調（x006d/x007b 並走の stale 上書き防止）。分散 TX 非保証とは別軸のローカル整合

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | done |
| 1 | protocol key + AeadKind + relay ACL | done |
| 2 | world DELETE + テスト | done |
| 3 | host despawn worker + Rapier remove + reseal | done |
| 4 | client Inventory Despawn UI | done |
| 5 | architecture / README 鮮度 | done |

## 完了条件

1. host が mesh `entity_id` を DELETE でき、revision が上がり entry が消える；不正 / user_root / 無 blake3 → 4xx
2. host role Desktop Inventory から Despawn → Manifest から消え、guest Frame remove で描画も消える
3. guest / 非 host の despawn 要求は拒否
4. Rapier に無い entity でも scene+reseal は適用（spawn の逆対称の警告のみ）
5. 保証表とコード一致；world / protocol 関連テスト緑

## やらない（本 Slice）

- put-back と Despawn の一体操作
- Prefab 多子 / XR Inventory
- Intent 経路
- BlobRef refcount GC
- `goal.md` 更新

← [目次](../README.md)
