# x0079 — DemoScene 共有テストベクタ

> **ID:** `x0079` · **slug:** `demoscene-shared-vector` · **status:** `done` · **area:** `[world, protocol, shared-sim, docs]` · **depends_on:** `[x0039, x0075]`

目標: improvement-plan **P2-11** — Elixir `World.DemoScene` と Rust ミラーの配置ずれを、同一の ManifestSeal 期待 blake3（共有テストベクタ）で機械検知する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-11** · [opus-specific-weaknesses-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| DemoScene entries + 固定 seal フィールドから両側が同じ blake3 を出す（CI で照合） | DemoScene 定義の単一ソース化（Elixir が SSoT、Rust はミラー） |
| `PROTOCOL_PIN` / loco / TRS / tint / coin blake3 のずれを hash 不一致で検知 | 実 JWT の HostAttestation 検証（ベクタの attestation はプレースホルダ） |
| host `demo_entries_for_test` が shared-sim ミラーを経由 | UGC scene・guest 配布・collider scale |

## 設計

1. **`alchemy_protocol::demo_scene_vector`**: 固定 `room_id` / `host_generation` / `host_pub_hex` / attestation + 期待 `DEMO_SCENE_VECTOR_BLAKE3`
2. **`alchemy_shared_sim::demo::demo_scene_entries`**: Elixir `DemoScene` の Rust ミラー。host テストはこれを呼ぶ
3. **`World.DemoSceneVector`**: 同定数の Elixir 側。`CanonicalManifest.build_payload` + `ManifestSealHash` で照合
4. DemoScene 変更時は下記 **再生成手順** で JSON / hex / yaw bits を更新する（手作業コピペはあるが、出力元は定型スクリプト）

## 再生成手順

手作業前提のコピペはあるが、**正本ファイルと hex の算出はスクリプト化済み**。

```bash
# cwd 不問（出力は scripts/ からの相対で repo の protocol/testdata へ）
mix run world-server/scripts/regen_demo_scene_vector.exs
# または cd world-server && mix run scripts/regen_demo_scene_vector.exs
```

スクリプトが行うこと:

1. `DemoSceneVector.payload/0`（`DemoScene` + 固定 seal フィールド）を組み立てる
2. `__DIR__` 基準で `protocol/testdata/demo_scene_seal.json` を上書き
3. Port 経由の `blake3=` を stdout に出す
4. `cargo` があれば `demo-scene-vector-yaw-bits` で center box（entity 14）の paste-ready `YAW45_*` `from_bits` 行を出す（fixture JSON → Rust 正規化後。Elixir 小数ではない）

その後の人手:

1. `blake3=` → `World.DemoSceneVector` `@blake3` と `DEMO_SCENE_VECTOR_BLAKE3`
2. `YAW45_*` 行が変わっていれば → `alchemy_shared_sim::demo`（**`@yaw45_*` 小数のコピー禁止**）
3. `cargo test -p alchemy-protocol` / `-p alchemy-shared-sim`

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| auth/world/relay metrics + `jti` 相関 | 別 Slice |
| scene despawn | 別 Slice |
| DemoScene JSON の単一ファイル codegen | しない（hash 照合で足りる） |

## 完了条件

1. Elixir `World.DemoSceneVectorTest` が DemoScene → seal hash == 共有期待値
2. Rust `alchemy-shared-sim` が `demo_scene_entries` → seal hash == 同じ期待値
3. host `demo_entries_for_test` が shared-sim ミラーを使用（user_root tint 含む）
4. Slice / backlog / README 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | protocol 定数 + shared-sim ミラー | done |
| 2 | Elixir / Rust テスト | done |
| 3 | docs 鮮度 | done |

← [目次](../README.md)
