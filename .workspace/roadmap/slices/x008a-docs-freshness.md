# x008a — docs 鮮度（client / wan / world 保証表 / vision）

> **ID:** `x008a` · **slug:** `docs-freshness` · **status:** `done` · **area:** `[docs]` · **depends_on:** `[x0066, x0069, x007e, x0089]`  
> **優先度:** 評価 P2-10（衛生）

目標: architecture / WAN 設定コメント / vision をコードと保証表の現状に揃え、見出しだけの `vision.md` を解消する。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-10** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `client.md` チャネル表が `{wire_cap}` 付きキー式と一致 | 全 architecture ファイルの再監査 |
| `config.wan.json5` / `relay-server-wan.md` が第2壁 = Room 鍵 AEAD（x0069）・`wire_cap` はルーティングのみと明記 | zenohd 設定本体の挙動変更 |
| `world-server.md` 保証表に x0066（CSR→短命 mTLS）と x0069（`room_key` 配布）が載る | P0-2 以降の未実装保証の先取り |
| `protocol.md` / `relay-server.md` のキー式一覧が inventory を含む | vision の製品ロードマップ詳細；overview 図の再描画 |
| `vision.md` に最終目標の短文がある | 製品マーケ文言の確定；全 evaluation アーカイブの遡及更新 |

---

## 変更ファイル

| ファイル | 内容 |
|:---|:---|
| [`.workspace/0_doc/architecture/client.md`](../../0_doc/architecture/client.md) | チャネル表に `{wire_cap}`・inventory spawn/despawn |
| [`.workspace/0_doc/architecture/protocol.md`](../../0_doc/architecture/protocol.md) | キー式ヘルパに inventory |
| [`.workspace/0_doc/architecture/relay-server.md`](../../0_doc/architecture/relay-server.md) | MVP キー式に inventory |
| [`.workspace/0_doc/architecture/world-server.md`](../../0_doc/architecture/world-server.md) | x0066 / x0069 保証行・Token 応答 JSON・API 注記 |
| [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5) | 第2壁コメントを AEAD に訂正 |
| [`.workspace/0_doc/manual/relay-server-wan.md`](../../0_doc/manual/relay-server-wan.md) | L190 旧「第2壁 = wire_cap」を AEAD に訂正 |
| [`.workspace/vision.md`](../../vision.md) | 最終目標の短文 |
| [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) | P2-10 done |
| [evaluation / opus 評価](../../0_doc/evaluation/) | 鮮度項・集計を closed に整合 |
| [roadmap README](../README.md) | 目次 x008a・空き `x008b` |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 1 | client / wan / world 保証表の鮮度合わせ | **done** |
| 2 | vision 短文 | **done** |
| 3 | improvement-plan P2-10 + roadmap 目次 | **done** |
| 4 | レビュー指摘（集計・manual・キー式一覧・表の差分） | **done** |

---

## 完了条件

1. client / protocol / relay-server / world / wan コメント / wan manual が Room 鍵 AEAD・`{wire_cap}` キー式で矛盾しない
2. `vision.md` が見出しのみでない
3. improvement-plan P2-10 を done 表記
4. 本スライスにランタイムコード変更を含めない

← [目次](../README.md)
