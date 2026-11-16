# x00c8 — Spatial VCS オブジェクト codec（決定的 CBOR）

> **ID:** `x00c8` · **slug:** `vcs-object-codec` · **status:** `done` · **area:** `[protocol, docs]` · **depends_on:** `[x0062, x0075]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.1（VCS 本線の最初）  
> **後続:** DB 内容アドレス移行 + `promote_prefab` 置換 + assets Port 配線（次空き `x00c9` 想定）

目標: Tree / Commit の正規エンコード・厳格デコード・blake3 ID・ゴールデンベクタ・`vcs-object` Port バイナリを `protocol/` に置く。ローカルとサーバで同じ ID になるための単一実装（Apache-2.0）。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §5–§6（候補 B 採用）· §18.1

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 同一論理 Tree/Commit → 同一正規 CBOR バイト列 → 同一 blake3 | assets DB 移行・`promote_prefab` 置換 |
| 厳格デコードが非正規頭部・未知キー・余りバイト・意味違反を拒否 | Origin Cert v2 / visibility / push / fork / PR |
| TRS v1 = IEEE754 f32 LE 40 bytes（NaN / Inf / -0.0 拒否；encode は -0.0→+0.0） | f64 TRS・ワールド Scene Tree |
| CBOR major 1（負の整数）拒否。`created_at_ms` は非負 uint。ルートは `generation == 1` | 親子 generation 整合（親 lookup → x00c9） |
| 敵対的長さ引数は残りバイトで上限（OOM なし）。Port JSON の TRS は f32 LE b64 のみ | JSON 数値配列 TRS |
| `vcs-object --serve` が verify / encode を JSON で返す（不正は soft-error・生存） | assets-server Port プール配線 |
| 新規 Cargo / Hex 依存なし。`PROTOCOL_PIN` 非バンプ | client `vcs-store` / Save UI / Save As |

---

## 設計メモ

- 形式: 決定的 CBOR Alchemy プロファイル（整数キー・最短頭部・浮動小数タグ禁止）
- ID = `blake3(正規バイト列の全体)`。`kind`（Tree=1 / Commit=2）でハッシュ空間を分離
- Port framing は既存 `port_framing`（`{:packet, 4}`）を再利用。オブジェクト上限 1 MiB
- Elixir 側の Port ワーカーは本スライスに含めない（§18.2）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | `alchemy_protocol::vcs_object` + ゴールデンベクタ | done |
| 2 | `vcs-object` バイナリ + docs 鮮度 | done |

---

## 完了条件

1. ゴールデンベクタで固定入力 → 固定バイト列 → 固定 blake3
2. 非正規 CBOR・意味違反はすべて拒否
3. `vcs-object --serve` が verify / encode を返し、不正でも soft-error で生存
4. 新規依存なし、`PROTOCOL_PIN` 非バンプ、assets DB / promote 非変更
5. 本 Slice / README / draft §18 鮮度一致

## やらない

- market / License Token
- サーバ側 3-way マージ
- Prefab.Payload JSON の廃止（現行 Drive は当面そのまま）

← [目次](../README.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
