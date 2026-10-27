# x002f — P3 alchemy_auth_client + undocumented_unsafe

> **ID:** `x002f` · **slug:** `p3-auth-client-unsafe` · **status:** `done` · **area:** `[shared, world, assets, client, docs]` · **depends_on:** `[x002e]`

目標: 評価書 P3 の **JWKS 複製解消（P3-4）** と **`undocumented_unsafe_blocks` deny（P3-14）** を縦に潰す。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md) P3-4 / P3-14

## 含む / 含まない

| 含む | 含まない（後続） |
|:---|:---|
| P3-4 `shared/alchemy_auth_client`（JWKS cache + Verify） | host-server Rust JWKS / `alchemy_auth_client` Hex 公開 |
| world / assets の JWKS・Verify 複製を path dep に置換 | revocation feed |
| kid miss 時の fetch クールダウンを共有実装で維持 | shared-sim / EntityKind |
| P3-14 client workspace で `clippy::undocumented_unsafe_blocks = deny` | 全 crate の `forbid(unsafe_code)` |

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| P3-4 | shared | GenServer JWKS: `otp_app` + 登録名。URL / fetch_on_start は各 OTP app の env。`signer_for_kid` は unknown kid で cooldown 付き再取得 |
| P3-4 | world / assets | 薄い facade（既存 `World.Auth.Jwks` / `Assets.Auth.Jwks` 名を維持）または Application から共有モジュールを name 付き起動 |
| P3-14 | client | workspace lint deny。XR（loader / render_model / vk_loop）の欠落 Safety コメントを補充 |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| world と assets が同一 JWKS 実装を参照する | Hex 版リリース・semver 公開 |
| unknown kid 連打で auth JWKS へ 1s 未満の連続 fetch をしない（キャッシュ非空時） | 鍵ローテーションの即時伝播保証 |
| `cargo clippy -p client` が undocumented unsafe で落ちる | OpenXR/Vulkan FFI 自体の形式検証 |

## 成功条件

1. `World.Auth.Jwks` / `Assets.Auth.Jwks` に独立した fetch 実装が残っていない  
2. `alchemy_auth_client` に JWKS / Verify 単体テスト（cooldown 両方向・install_test・iss/aud/status）  
3. CI: `alchemy-auth-client-ci` が `shared/alchemy_auth_client` の `mix test` を回す（world/assets は path-dep コンパイルのみ）  
4. client workspace: `clippy::undocumented_unsafe_blocks = "deny"` で緑  

← [目次](../README.md)
