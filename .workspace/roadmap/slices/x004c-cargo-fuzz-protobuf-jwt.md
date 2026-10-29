# x004c — cargo-fuzz（protobuf / JWT）

> **ID:** `x004c` · **slug:** `cargo-fuzz-protobuf-jwt` · **status:** `done` · **area:** `[protocol, host, docs]` · **depends_on:** `[x003b, x0045]`  
> **優先度:** **P3-9**

目標: 非信頼入力の decode / Room Token verify を `cargo fuzz` で短時間スモークし、panic・OOM 手前の破綻を早期に拾う。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-9**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `protocol/fuzz`: Manifest / Frame / Presence / Input の decode → post-check（CI 既定 `-max_len=8192`） | **byte gate 超過**（`PayloadTooLarge`）— **単体テスト**（`check_payload_bytes` / `decode_*_checked`） |
| `host-server/fuzz`: `RoomTokenVerifier::verify`（**固定 PEM** JWKS・feature `fuzzing`） | HTTP JWKS・credential worker・暗号破りの網羅 |
| Linux CI / `make ci-fuzz` で各ターゲット短時間実行 | Windows MSVC 上の libFuzzer；長時間キャンペーン |
| seed corpus（`seed_*`）+ fuzz `Cargo.lock` をリポジトリに含める | 発見 corpus の恒久保管 |
| `ci-fuzz.sh` は実行ビット付き（`100755`） | release / ZIP での `fuzzing` feature 有効化 |

## 実行

```bash
# 依存（人確認済み想定）: cargo-fuzz → libfuzzer-sys（fuzz クレートのみ・本体 workspace 外）
cargo install cargo-fuzz
rustup toolchain install nightly -c rust-src

# protocol（各ターゲット）
cd protocol && cargo +nightly fuzz run decode_frame -- -max_total_time=60

# host JWT
cd client/host-server && cargo +nightly fuzz run room_token_verify -- -max_total_time=60

# CI 相当スモーク
make ci-fuzz
# または: ./.scripts/ci-fuzz.sh [protocol|host|all]
```

seed 再生成（**cargo-fuzz クレート外**の helper）:

```bash
cargo run --bin gen_fuzz_corpus --manifest-path protocol/fuzz-tools/Cargo.toml
cargo run --bin gen_room_token_seed --manifest-path client/host-server/fuzz-tools/Cargo.toml
```

## 完了条件

- protocol 4 ターゲット + host `room_token_verify` が短時間で完走（panic なし；host は固定 PEM で鍵生成コストなし）
- 保証表（architecture/protocol.md・本スライス）とコード一致
- `make ci` 本体を肥大化させない（fuzz は別ゲート）

← [目次](../README.md)
