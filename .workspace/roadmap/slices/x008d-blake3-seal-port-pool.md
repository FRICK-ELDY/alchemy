# x008d — BLAKE3 / seal Port プール（PartitionSupervisor）

> **ID:** `x008d` · **slug:** `blake3-seal-port-pool` · **status:** `done` · **area:** `[assets, world, docs]` · **depends_on:** `[x0075, x008b]`  
> **優先度:** 評価 P2-1（負債・衛生）

目標: assets の BLAKE3 と world の seal-hash を、単一 GenServer / Port 直列から **`PartitionSupervisor` による N 本プール**へ上げる。新規 Hex 依存は追加しない。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-1** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 既定は **起動ホスト** の `System.schedulers_online/0`（`runtime.exs`；非 test のみ env 上書き）。`config.exs` には置かない（release ビルドホスト焼き付き回避）。**test は env 無視**（`test.exs` の 2 を hermetic に保つ） | パーティション数を超える真の並列；ビルド時 CPU 数の固定；test での env 強制 |
| 呼び出しは `{:via, PartitionSupervisor, {name, key}}` で振り分け（整数 key = rem） | 完全公平スケジューリング・最短待ち保証 |
| `ready?/0` は全パーティションの Port を `:persistent_term` + `Port.info` で非ブロッキング検査。happy-path reopen は新 Port を先に publish。terminate は state Port と PT 上の別 Port の両方を close（mid-reopen kill の孤児防止） | open 失敗・terminate・外部 close〜handle_info 前の `/ready` false |
| ファサード維持: `Assets.Blake3.hash_hex/1`（raise・`:noproc`/timeout 含む）/ `World.ManifestSealHash.blake3_hex/1`（`{:error,_}`） | Worker エラー型の統一 |
| framing / soft-error / reopen は x0075 と同じ | ストリーミング NIF（旧 P3-7） |

---

## 設計（要点）

```
Application
  └─ PartitionSupervisor (Assets.Blake3.Partitioner / World.ManifestSealHash.Partitioner)
        ├─ Worker partition 0  → Port 0
        ├─ Worker partition 1  → Port 1
        └─ …
```

- `:with_arguments` で各 Worker に `:partition` を注入。Port は `{Mod, :port, partition}` で `:persistent_term` 公開
- 振り分け key: `:erlang.unique_integer([:positive])`（整数 rem でラウンドロビン相当）
- 設定: `runtime.exs`（非 test: 起動時 `schedulers_online` / trimmed env）+ `test.exs`（2・env 無視）。bin 解決は protocol/`CARGO_TARGET_DIR` を PATH より優先（stale `cargo install` の `--serve` 欠落対策；release は runtime の bin パスが勝つ）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | assets Blake3 PartitionSupervisor | **done** |
| 2 | world ManifestSealHash PartitionSupervisor | **done** |
| 3 | テスト + architecture / improvement done | **done** |

---

## 完了条件

1. 同時ハッシュが複数 Port に載る（単一 GenServer 直列が消える）
2. `/ready` がプール全体を見て 503 できる（x008b 契約維持）
3. 既存ベクタ / reopen / soft-error テスト緑
4. improvement P2-1 を done 表記

## やらない（本 Slice）

- poolboy / 新 Hex
- BLAKE3 ストリーミング NIF
- Origin Cert content hash（P2-4）→ [x0096](x0096-origin-cert-content-hash.md)

## 関連

- [x0075-blake3-seal-hash-port.md](x0075-blake3-seal-hash-port.md)
- [x008b-ready-and-blob-nosniff.md](x008b-ready-and-blob-nosniff.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
