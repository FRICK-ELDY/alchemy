# x0075 — BLAKE3 / seal-hash 常駐 Port

> **ID:** `x0075` · **slug:** `blake3-seal-hash-port` · **status:** `done` · **area:** `[assets, world, protocol, ci]` · **depends_on:** `[x0073, x0074, x0039]`

目標: assets の BLAKE3 と world の seal-hash を、毎回 `System.cmd` + 内容 tempfile から **常駐 Port（`{:packet, 4}`）+ GenServer** に置き換える。NIF / 新 Hex は使わない。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P0-次 / P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-2**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 起動後 1 Port を再利用し、リクエスト毎の OS プロセス起動・内容 tempfile をしない | 並行ハッシュのスループット |
| framing + hex 検証。失敗は fail-closed（空 hash を返さない） | Port 再起動窓のゼロ失敗 |
| seal は従来と同じ `manifest_seal_blake3_hex` 経路 | NIF / ストリーミング API |
| Windows 含む（FIFO 非依存・packet 4） | `b3sum` 互換 CLI の永続サポート |

## 設計

1. **protocol** `blake3-hash --serve` / `manifest-seal-hash --serve`: u32 BE length-prefixed stdin/stdout（soft error 接頭辞 `!`）
2. **`Assets.Blake3.Worker`** / **`World.ManifestSealHash.Worker`**: named GenServer が Port 所有。`handle_call` 内で応答待ち（呼び出しは GenServer キューで直列化）
3. ファサード API 維持: `Assets.Blake3.hash_hex/1`・`World.ManifestSealHash.blake3_hex/1`
4. assets CI/Docker から `b3sum` を外し、自前 bin に置換

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| host `room_token.rs` / client `main.rs` 分割 | 別 Slice |
| bind 送信元分割・mTLS temp・DemoScene ベクタ・metrics+jti | 別 Slice |
| BLAKE3 ストリーミング NIF（旧 P3-7） | マージしない |

## 完了条件

1. assets: `b3sum` / 内容 tempfile 経路が消える。既知ベクタテスト緑
2. world: seal tempfile + 毎回 `System.cmd` が消える。canonical / manifest_seal テスト緑
3. CI/Docker が自前 bin のみ
4. slice / backlog / README 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | protocol bins（`--serve` + framing） | done |
| 2 | assets Worker + CI/Docker | done |
| 3 | world Worker + CI | done |
| 4 | docs + テスト確認 | done |

← [目次](../README.md)
