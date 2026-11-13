# Alchemy — 配信の空間インデックス（Spatial Interest）正本

* **種別**: 規模本線の仕様正本（アウトライン）。実装コード・新スライス実行は含まない
* **改善計画上の位置**: [improvement-plan-2026-11-12.md](../0_doc/evaluation/improvement-plan-2026-11-12.md) **P3-7**
* **品質本線（物理・状態）**: [mvcc_xpbd.md](mvcc_xpbd.md) Phase 1–3（dirty デルタ・SoA ページが前提）
* **現行配送契約**: [protocol/peer-delivery.md](../../protocol/peer-delivery.md)（x0024）· [architecture/client.md](../0_doc/architecture/client.md) · [architecture/protocol.md](../0_doc/architecture/protocol.md)
* **関連**: x0067（非対称 ACL）· x0069（Room 鍵 AEAD）· `HotPhysicsPage.cell_origin`（[mvcc_xpbd.md](mvcc_xpbd.md) §4.3）

> **注記（番号衝突）**: 本 P3-7（Spatial Interest）は、roadmap README に残る旧メモ「P3-7 BLAKE3 ストリーミングはマージしない」とは**無関係**。BLAKE3 ストリーミング禁止は引き続き有効。

---

## 0. なぜ第一級仕様か

[mvcc_xpbd.md](mvcc_xpbd.md) は近傍物理品質・ゼロアロ・handover 速度復元・GPU 補間の**品質本線**である。  
同一 Room の大規模 CCU（horizon として 1,000 人）は、dirty bitmask や量子化だけでは届かない。現行は Room 単位の単一 `…/{wire_cap}/frame` を全員が購読する**フルメッシュ**であり、リレー送信と各クライアント受信が物理より先に死ぬ。

**規模の主戦場は「誰に・何を・何 Hz で送るか」（interest）である。** 本ドキュメントがその正本。

```text
品質本線: mvcc_xpbd Phase 1–3
  → dirty / AEAD デルタが使える
規模本線: spatial_interest（本ファイル）
  → セル購読分割 + avatar LOD
  → 実測ゲート 32 → 64 → 128
  → 1000 人は horizon（到達保証ではない）
```

---

## 1. 問題定義（現行）

| 現状 | 帰結 |
|:---|:---|
| キー式 `alchemy/…/{wire_cap}/frame` が Room 全体 1 本（[client.md](../0_doc/architecture/client.md)） | 参加者 N が増えるとファンアウトが O(N) ペイロード × O(N) 購読者 |
| host → guest の latest デルタ（x0024） | 欠落時 snapshot は正しいが、**興味範囲の縮小は契約に無い** |
| MVCC dirty + Smallest-Three | フルメッシュ前提の**ペイロード極小化**に効く。N 人同期トポロジ自体は解かない |
| `USER_ROOT_SLOTS_MAX` DoS 天井 32 | 品質本線の到達点と 1000 人 horizon の間に段階ゲートが必要 |

粗い帯域感（アバター head+hands ≈ 3 entity、全員毎ティック更新・理想値）:

* 1000 × 3 × ~40 B ≈ 120 KB/tick → 30 Hz で受信側 ~3.6 MB/s
* 1000 購読者が同一フルストリーム → リレー送信は GB/s 級になりうる

---

## 2. 保証する / しない

| 保証する（目標・実装スライス後） | 保証しない（当面） |
|:---|:---|
| クライアントは**近傍セル群**のみ高頻度 `frame` 相当を購読できる | Phase 文書の存在だけで 1000 人 CCU を保証すること |
| 遠方エンティティは**低頻度 LOD**（間引き / 自由度削減）で届く | 1 host 上での 1000 人**相互** XPBD・全員接触 |
| 既存 Room AEAD（x0069）· wire_cap ルーティング · delivery-class（x0024）を壊さない | 確実配送 ACK / event ID チャネル |
| 物理ページの空間セルと配信キーの空間単位を**同系**に保つ | `wire_cap` トピック名の秘匿 |
| 独自トランスポートを新設しない（Zenoh パイプライン維持） | マルチ権威セルの本番移譲・セル間 handover の完成 |
| | 音声・メッシュ・アニメ予算の本仕様化（別予算・非保証） |
| | verifier Spot-Check / billing / market（独立） |

実装スライス着手時に、[protocol.md](../0_doc/architecture/protocol.md) · [client.md](../0_doc/architecture/client.md) · [peer-delivery.md](../../protocol/peer-delivery.md) の保証表を**本契約に合わせて更新**する（本アウトライン作成時点では architecture 本改訂を行わない）。

---

## 3. 空間単位（物理ページと配信の接合）

| 層 | 単位 | 役割 |
|:---|:---|:---|
| 物理（品質本線） | `HotPhysicsPage` + `cell_origin_* : f64`（局所 `f32` ±32 m） | XPBD・MVCC・GPU 転送 |
| 配信（規模本線） | **同一系のセル ID**（`cell_id: [i32; 3]` または origin から導出） | 購読・publish の interest キー |
| 権威（当面） | **1 Room = 1 host** | セルは購読分割の単位であり、権威分割の単位ではない |

`cell_origin` を interest の第一級キーに昇格させる。物理ページ設計を捨てて別グリッドを発明しない。

---

## 4. キー契約の方向（設計決定）

1. **独自トランスポート禁止**（[mvcc_xpbd.md](mvcc_xpbd.md) 🚫1 と同型）。`relay-server` / `relay-agent` / `wire_cap` ACL / Room AEAD を通す。
2. 現行 `…/{wire_cap}/frame`（および必要なら `snapshot`）を**即座に廃棄しない**。段階拡張する:
   - **方向 A**: セル修飾サフィックス（例: `…/frame/cell/{cx}_{cy}_{cz}`）
   - **方向 B**: cell 別キー族ヘルパを `protocol/` に追加し、Room 全体キーは互換・集約用に残す
3. **x0067 非対称 ACL** と **AEAD AAD** のキー文字列互換／世代は、**後続実装スライスで契約を固定**する。本正本では具体キー文字列を確定しない（破壊的変更をアウトライン単独で起こさない）。
4. delivery-class（latest / snapshot / best-effort）の語彙は [peer-delivery.md](../../protocol/peer-delivery.md) を継承し、セル分割後も「frame = latest・欠落は snapshot」を維持する。

---

## 5. Avatar sync LOD

| 帯 | 目安 | 同期内容（目標） |
|:---|:---|:---|
| 近傍 | 同一セル + 隣接セル | フル TRS（必要なら hands）高 Hz・dirty デルタ |
| 中距離 | 数セル先 | 間引き Hz・自由度削減（例: root のみ） |
| 遠方 | AOI 外 | presence 級（存在・粗位置）または非購読 |

音声・アバターメッシュ・表情は**本正本の保証外**（別予算）。物理 Grab / 接触は近傍帯に閉じる（品質本線の XPBD と整合）。

---

## 6. 権威モデル

| 段階 | モデル | 規模への効き |
|:---|:---|:---|
| **当面（本正本の実装第一波）** | 1 Room = 1 untrusted host。セルは**購読・publish 分割**のみ | フルメッシュ解除が主効果 |
| **後段 horizon** | 論理ワールド複数セルのまま、必要ならマルチ権威 | host CPU 限界を超えるときのみ |

メガ空間は「権威を先に割る」より「**購読を先に割る**」。マルチ権威セル間の一貫性・移譲は更に後段とし、品質本線の handover（速度復元）と混同しない。

---

## 7. マイルストーン（実測ゲート）

現行 DoS 天井 `USER_ROOT_SLOTS_MAX = 32` を起点にする。各段で **帯域・host CPU・client 受信**を測り、次段の天井拡大条件にする。

| ゲート | CCU 目安 | 合格の意味（例） |
|:---|:---|:---|
| G0 | ≤32 | 現行契約 + MVCC Phase 3 デルタで安定 |
| G1 | 64 | interest（近傍セル購読）無しでは不合格になりうることを計測で示すか、interest 試作で通過 |
| G2 | 128 | cell 購読 + avatar LOD が本線。フルメッシュ禁止を運用で固定 |
| Horizon | 1000 | interest・予算・（必要なら）権威拡張が揃った**将来目標**。到達保証ではない |

「MVCC Phase 完了 = 1000 人」と書かない。規模 KPI は本ファイル、品質 KPI は [mvcc_xpbd.md](mvcc_xpbd.md)。

---

## 8. 依存と着手ゲート

| 依存 | 理由 |
|:---|:---|
| [mvcc_xpbd.md](mvcc_xpbd.md) **Phase 3** DoD Green | dirty デルタ・host 権威ループ・AEAD 配信が品質として使えること |
| x0024 delivery-class | latest / snapshot 再同期語彙 |
| x0067 / x0069 | ACL・部屋機密を壊さない拡張 |
| 独立 | P3-1 verifier · P3-3 S2S · P3-4 billing · Formula / OSC |

| 作業 | 条件 |
|:---|:---|
| **本ファイル（アウトライン正本）** | いつでも。MVCC Phase と**並走可** |
| **実装スライス群**（採番は着手時。`x00a4` は予約しない） | **MVCC Phase 3 DoD Green 後**。protocol / zenoh ACL / client 購読の否定系を含める |

---

## 9. やらないこと

* 独自 UDP / quinn / WebTransport で帯域を稼ぐ
* 検証スキップ・Room AEAD / ManifestSeal バイパス
* MVCC だけで大規模 CCU を宣言する文書・PR 説明
* フルメッシュのまま Phase を増やして「スケールした」ことにする
* 本アウトラインだけで architecture 保証表やキーヘルパを破壊的に書き換える（実装スライスで契約固定）
