# x009a — XR loader tempfile 純粋化（P2-12）

> **ID:** `x009a` · **slug:** `xr-loader-tempfile-pure` · **status:** `done` · **area:** `[client]` · **depends_on:** `[x0008]`  
> **優先度:** improvement **P2-12**（Windows workspace test flake 除去）

目標: Steam `libraryfolders.vdf` の path 抽出を **FS 非依存の純粋 parse** に切り出し、`Path::is_dir` フィルタを別関数・別テストにする。固定 temp 名での create/write/remove をテストから消し、Windows 並列で一過性失敗しないようにする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-12** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md) · [gpt-specific-proposals-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-proposals-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| VDF `"path"` 行の抽出は `parse_library_folder_paths(&str)`（IO なし） | Steam VDF 全方言・ネスト構造の完全パース |
| 既存ディレクトリ選別は `filter_existing_dirs`（`is_dir` のみ） | 並列 FS 競合の数学的ゼロ証明 |
| `parse_library_folders(Path)` は read → parse → filter の合成のみ | OpenXR 実ロード経路の変更 |
| 単体テストは固定 temp 名を使わない（一意 subdir = pid+nanos） | 新規依存（`tempfile` / `uuid`）の追加 |
| `parse_library_folders` の read→parse→filter 結合を一意 tempfile で回帰 | — |
| malformed `"path"` 行は行単位 skip（後続の有効 path は残す） | （旧）malformed 行で関数全体 `None` になる挙動 |

---

## 設計（要点）

```
libraryfolders.vdf (Path)
        │ read_to_string
        ▼
parse_library_folder_paths(&str)  → Vec<PathBuf>   // pure
        │
        ▼
filter_existing_dirs(...)         → Vec<PathBuf>   // is_dir only
```

- 本番 `candidate_loader_paths` の呼び出し口は `parse_library_folders` のまま
- テスト: parse は文字列 fixture、filter は `temp_dir()` + 一意な非存在パス、結合は一意 tempfile で read→parse→filter

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | parse / filter 分離 + 純粋テスト | **done** |

---

## 完了条件

1. `parse_library_folder_paths` が IO なしで path を返す単体テスト（malformed 行は skip）
2. `filter_existing_dirs` が存在する dir のみ残す単体テスト（一意な非存在パス）
3. `parse_library_folders` の read→parse→filter 結合テスト（一意 tempfile）
4. `cargo test -p client -- surface::xr::loader::tests` 緑
5. improvement P2-12 を done 表記

## やらない（本 Slice）／次

- USER_ROOT 帯 Manifest（P2-13）は [x009b](x009b-user-root-band-manifest.md)；bind 全体上限（P2-14）は [x009c](x009c-bind-global-session-pub.md)
- OpenXR loader 探索ロジック自体の拡張

## 関連

- [x0008-openxr-dash-exit.md](x0008-openxr-dash-exit.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)
- `client/app/src/surface/xr/loader.rs`

← [目次](../README.md)
