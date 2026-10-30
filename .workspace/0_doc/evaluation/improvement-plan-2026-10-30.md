# Alchemy — 改善提案書（まとめ / 2026-10-30）

> 根拠: [specific-weaknesses-2026-10-30.md](specific-weaknesses-2026-10-30.md)  
> 前回計画: [archive/2026-10-28/improvement-plan-2026-10-28.md](archive/2026-10-28/improvement-plan-2026-10-28.md)  
> 目的: 採用マイナス ~-28 を潰し、正味 ~+77 をさらに押し上げる。

---

## 方針

1. **Relay 門番を全体水準に合わせる**（NotAfter・JWKS hard・issuance bound・CSR・CI）— 「短命・失効可能・bounded」
2. **部屋隔離の文書とコードを一致させる**（短期は保証表修正、本対応は L7 / AEAD）+ WAN 否定系
3. **CAS access を物理から権利側へ**（policy squatting 解消）
4. **client trust / relay を E2E・CI に載せる** — 新クレート漏れの再発防止
5. **inventory ladder（x0054〜）は門番を閉じた後** — 機密性が乗る前提

前回 P0〜P3 の大半はコードで閉じた。残課題と新規をこの版に載せ替える。

---

## P0 — 今すぐ（Relay / WAN / CAS）

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | 証明書 NotAfter = `expires_in` | -3 | OpenSSL `-not_after` / `ca -enddate` 等で秒単位。応答 TTL と一致 | 発行直後の NotAfter が ≈300s。漏えい鍵が 24h 残らないテスト |
| P0-2 | Allocation hard JWKS / body / thread / rate / jti | -3 | hard TTL、unknown-kid cooldown、Content-Length 事前上限、発行 semaphore、jti 短期 dedupe | oversize / stale JWKS / 連打で OOM・無制限 spawn しないテスト |
| P0-3 | 部屋隔離: 保証表修正 → 本対応 | -3 | 短期: trust-boundary / zenoh-router / x004e で `wire_cap` 隔離を非保証へ。本対応: Relay L7 または Room 鍵 AEAD（依存は人に回す） | guest が `*/*/manifest` subscribe しても他部屋が届かない否定系（WAN 構成） |
| P0-4 | 秘密鍵をネットワークに出さない | -2 | client CSR。relay は署名のみ。Tunnel/TLS 背後。手順書から `0.0.0.0` cleartext を外す | 応答に `key_pem` が無い。手順書どおりでも平文鍵が WAN に流れない |
| P0-5 | CAS access を Grant/ref へ | -3 | `CasObject` は bytes のみ。access は ref/grant。先着固定を廃止 | 同一 hash で公開 mesh と private inventory を両立するテスト |

---

## P1 — CI・E2E・可用性

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | relay-agent を CI / `make ci` / cargo deny へ | -2 | fmt/clippy/test job。NotAfter・JWKS・body の契約テスト | `ci.sh` 緑に relay を含む。テスト >0 |
| P1-2 | headless client trust consumer + WAN E2E | -2 | app と同じ gate 関数で偽 seal / stale gen / oversize。compose に mTLS zenohd | 偽 Manifest で joined 維持・state 不変。wildcard 否定系 |
| P1-3 | world rate limit + Room 上限 | -2 | `Auth.RateLimit` を shared 化。Room per-owner cap。P3-7 から BLAKE3 と切り離す | 連打で SQLite / auth S2S が増幅しない |
| P1-4 | launcher discovery endpoint（IPC 更新） | -1 | 固定 loopback で host generation/address を client へ再通知 | host restart 後も Exit Client+Host が届く |
| P1-5 | README / client.md / vision 鮮度 | -1 | 現状を x0052・x004e/f・次 ID x0066・mTLS に。vision は書くか削除 | 入口が architecture と矛盾しない |

---

## P2 — 負債・衛生

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P2-1 | reserved reaper / orphan CAS 方針 | -1 | `reserved_at` + 定期回収 |
| P2-2 | BLAKE3 / seal-hash 常駐 Port | -1 | CLI 毎回起動を廃止（NIF 回避） |
| P2-3 | auth Keys の `:persistent_term` | -1 | GenServer 直列化解消 |
| P2-4 | host `room_token.rs` / client `main.rs` 分割 | -1 | 保証単位でファイル分割 |
| P2-5 | bind 全体バケットの送信元分割 | -1 | 未知鍵洪水での入室 DoS 緩和 |
| P2-6 | tos_agreed UI 同意 | -1 | 固定 true 廃止 |
| P2-7 | prod compose BIND_ADDR 既定 loopback | -1 | postgres と同様に絞る |
| P2-8 | cargo deny を launcher / relay にも | -1 | 独立 lock を監査 |
| P2-9 | mTLS 鍵 temp の権限・掃除（Windows） | -1 | ACL + 期限切れ削除 |
| P2-10 | 腐った smoke 削除 or e2e-probe 吸収 | -1 | 現行ログと乖離する grep を消す |
| P2-11 | DemoScene 共有テストベクタ | -1 | Elixir/Rust 同一期待 hash |
| P2-12 | auth/world/relay metrics + `jti` 相関 | -1 | loopback 拡張 |

---

## P3 — アーキテクチャ（スライス推奨）

| ID | 項目 | 根拠 | 備考 |
|:---|:---|:---|:---|
| P3-1 | inventory ladder x0054〜x0065 | 提案 | **P0 部屋隔離の後**が望ましい |
| P3-2 | XR↔Desktop hot switch x0053 | 提案 | planned 継続 |
| P3-3 | verifier Spot-Check | 目標 | canonical seal / gen-rev 先行条件 **満たした** |
| P3-4 | BlobGrant 案 B（GET 常時 Grant） | 目標 | P0-5 と連続 |
| P3-5 | Formula / Wake / OSC / LOD / audio | 目標 | 減点しない |
| P3-6 | billing / market / Relay QoS 経済圏 | 目標 | ディレクトリ未設置 |

---

## 前回計画のクローズ状況（要約）

| 前回帯 | 状態 |
|:---|:---|
| P0 leave / seal / replay / decode / prod / world DB / assets required mesh | **完了** |
| P1 GET 文書 / CAS ref / credential+network worker / cnf / rate+JWKS / Grab clamp | **完了**（GET 実装は grant_required opt-in。access 配置は今回 P0-5） |
| P2 mTLS ACL / internal 404 / client JWKS cooldown | **方向完了・実効は今回 P0 へ** |
| P3 adversarial / launcher CI / placeholder / metrics / docs / ClientIp / fuzz / USER_ROOT / Room Token 否定系 | **大半完了**（adversarial は host のみ→今回 P1-2、BLAKE3 は見送り継続） |
| P4 XR wgpu / BlobGrant MVP / inventory ladder docs | **wgpu・BlobGrant MVP・ladder 文書完了**。hot switch / verifier / 経済圏は残 |

---

## 推奨スライス順

1. **relay 契約一括**（P0-1 + P0-2 + P1-1）— NotAfter / hard JWKS / bound / CI
2. **鍵配布 CSR + 公開面**（P0-4）
3. **部屋隔離**（P0-3 短期文書 → 本対応 + P1-2 WAN E2E）
4. **CAS access 移設**（P0-5）→ その後 inventory x0054
5. **world RL / README / launcher discovery**（P1-3〜5）を並行

スライス完了条件に必ず含める:

- **新クレートは CI・cargo deny・テスト >0**
- **新しい保証は否定系 E2E で確認**
- **保証表とコードが一致**（過大主張を残さない）
