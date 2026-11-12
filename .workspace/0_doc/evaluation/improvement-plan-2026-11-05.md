# Alchemy — 改善提案書（まとめ / 2026-11-05）

> 根拠: [specific-weaknesses-2026-11-05.md](specific-weaknesses-2026-11-05.md)  
> 前回計画: [archive/2026-10-30/improvement-plan-2026-10-30.md](archive/2026-10-30/improvement-plan-2026-10-30.md)  
> 目的: 採用マイナス ~-24 を潰し、正味 ~+94 をさらに押し上げる。

---

## 方針

1. **制御面から bearer Room Token を排除**（session 署名 + world PoP）— Intent と同水準
2. **Room 鍵に寿命を与える**（epoch / visibility・kick ローテ）— AEAD の revoke 境界
3. **canonical scene を CAS 更新にする** — 定義元の欠落を止める
4. **互換 Agent の `key_pem` を閉じる** — 正経路へ一本化
5. **host RL / GLB budget / assets test hermetic** — 門番衛生

前回 P0（Relay NotAfter / JWKS / 部屋隔離本対応 / CSR 正経路 / CAS access）はコードで閉じた。残課題と新規をこの版に載せ替える。

---

## P0 — 今すぐ（Token・鍵・scene）

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | 制御面を session 署名 + world PoP | -3 | spawn/despawn から `room_token` フィールド削除。Intent と同じ Ed25519。`host_keys`/seal/scene に `cnf.spk` 署名ヘッダ | Token が Zenoh に載らない。盗んだ host Token だけでは host_keys が 401。否定系 E2E |
| P0-2 | Room key epoch / revoke | -3 | `room_key_epoch`。visibility 縮小・kick・定期で回転。AAD+Token に epoch。猶予=Token TTL | 旧参加者の鍵で新 epoch 平文が読めない否定系。保証表に寿命を明記 |
| P0-3 | Scene revision CAS | -3 | `expected_revision` + `UPDATE … WHERE revision=?`。host は 409 再試行 | 同時 append+delete で欠落しない統合テスト — **x0082 done** |
| P0-4 | relay-agent CSR-only or 退役 | -2 | `csr_pem` 必須・`key_pem` 削除。非 loopback cleartext は起動拒否。不要なら allocate 廃止 | 応答に秘密鍵が無い。手順書も CSR のみ — **[x0083](../../roadmap/slices/x0083-relay-agent-csr-only.md) done** |

---

## P1 — 門番衛生・資源・E2E

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | host-token 操作別 rate limit | -2 | `room_id+jti` / IP。seal / scene / PlaceRead で上限分離 | 連打で seal Port / DB が増幅しない — **[x0084](../../roadmap/slices/x0084-host-token-rate-limit.md) done** |
| P1-2 | GLB decode budget | -2 | 頂点・index・pixel・展開 bytes ハード上限。超過は placeholder | 巨大/爆弾 fixture で OOM しないテスト — **[x0085](../../roadmap/slices/x0085-glb-decode-budget.md) done** |
| P1-3 | assets test hermetic | -1 | test で reaper/GC `start: false`。CAS は `use …, cas: true`（tmp_dir） | 既定並列でも 0 failures が安定 — **[x0086](../../roadmap/slices/x0086-assets-test-hermetic.md) done** |
| P1-4 | 実 host WAN 一往復 E2E | -1 | CSR→credential worker→signed Frame→trust consumer | no-cert / guest sim put / 旧 epoch 否定 — **[x0087](../../roadmap/slices/x0087-wan-host-e2e.md) done** |
| P1-5 | Allocation 主体別 throughput | -1 | sub/IP 短窓・長窓 bucket + metrics | 正規 jti 連打でも CPU 飽和しない — **[x0088](../../roadmap/slices/x0088-allocation-subject-throughput.md) done** |

---

## P2 — 負債・衛生

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P2-1 | BLAKE3 / seal Port プール | -1 | `PartitionSupervisor`（新規依存なし） — **[x008d](../../roadmap/slices/x008d-blake3-seal-port-pool.md) done** |
| P2-2 | `/ready`（Repo・Keys・Port） | -1 | health と分離 — **[x008b](../../roadmap/slices/x008b-ready-and-blob-nosniff.md) done** |
| P2-3 | mime を Ref 側 or octet-stream+nosniff | -1 | 型混同の足場を消す — **[x008b](../../roadmap/slices/x008b-ready-and-blob-nosniff.md) done** |
| P2-4 | Origin Cert に tree/content hash | -1 | 連合前にペイロード形を固定 |
| P2-5 | 共有 JWKS 失敗クールダウン | -1 | host/relay と同型 — **[x008c](../../roadmap/slices/x008c-shared-jwks-fail-cooldown.md) done** |
| P2-6 | host `main` tick 切り出し + pose ダブルバッファ | -1 | FrameBridge 影並走は **[x008f](../../roadmap/slices/x008f-host-frame-bridge.md) done**；XPBD wire 権威は **[x0093](../../roadmap/slices/x0093-xpbd-wire-authority.md) done**；wire pose ダブルバッファは **[x0095](../../roadmap/slices/x0095-wire-pose-double-buffer.md) done**（`HostLoop::tick` 切り出し全体は未） |
| P2-7 | take-out を guest+session / owner_sub 判定へ | -1 | P0-1 と一体 |
| P2-8 | 腐った smoke 削除 or probe 吸収 | -1 | banner-grep 削除・e2e-probe へ吸収 — **[x0089](../../roadmap/slices/x0089-rotten-smoke-shared-ci.md) done** |
| P2-9 | `ci.sh` に `shared/alchemy_*` | -1 | `ci.sh shared` / `all` — **[x0089](../../roadmap/slices/x0089-rotten-smoke-shared-ci.md) done** |
| P2-10 | client.md / wan コメント / world 保証表 / vision | -1 | 書くか削除 — **[x008a](../../roadmap/slices/x008a-docs-freshness.md) done** |
| P2-11 | Room 上限の厳密化 | -1 | quota row / serializable |
| P2-12 | XR loader tempfile 純粋化 | -1 | Windows flake 除去 |
| P2-13 | USER_ROOT 帯を Manifest 配 | -1 | PROTOCOL_PIN 更新が必要なら別スライス |
| P2-14 | bind 全体上限 + session_pub 起点 | -1 | 自己申告 ID 緩和 |

---

## P3 — アーキテクチャ（スライス推奨）

| ID | 項目 | 根拠 | 備考 |
|:---|:---|:---|:---|
| P3-1 | mTLS の room 束縛 / L7 | 提案 | zenohd 静的 ACL の限界 |
| P3-2 | verifier Spot-Check orchestration | 目標 | Noop 配線のみ OSS |
| P3-3 | Frame 補間 / jitter buffer | 提案 | |
| P3-4 | launcher deep link / handoff | 目標 | |
| P3-5 | billing / market / Relay QoS | 目標 | ディレクトリ未設置 |
| P3-6 | Formula / Wake / OSC / LOD / audio | 目標 | 減点しない |

---

## 前回計画のクローズ状況（要約）

| 前回帯 | 状態 |
|:---|:---|
| P0-1 NotAfter = expires_in | **完了** |
| P0-2 Allocation hard JWKS / body / thread / jti | **完了** |
| P0-3 部屋隔離（AEAD 本対応 + 保証表） | **完了**（revoke 寿命は今回 P0-2） |
| P0-4 秘密鍵をネットワークに出さない | **完了** — world CSR + Agent CSR-only（x0083） |
| P0-5 CAS access → Grant/ref | **完了** |
| P1-1 relay CI | **完了** |
| P1-2 headless trust + WAN E2E | **方向完了** — 実 host 一往復は **x0087**（P1-4）done |
| P1-3 world RL + Room 上限 | **部分完了** — auth scope done。host-token は今回 P1-1 |
| P1-4 launcher discovery | **完了** |
| P1-5 README 鮮度 | **完了** — README done。他 docs は **x008a**（P2-10）done |
| P2 x0074〜x0079 | **完了**（Port 直列・smoke 等は残） |

---

## 推奨スライス順

1. **制御面 + PoP**（P0-1 + P2-7）— Token をバスから外す
2. **Room key epoch**（P0-2）— 保証表更新を同 PR に含める
3. **Scene CAS**（P0-3）— inventory 同時更新の否定系
4. ~~**Agent CSR-only**（P0-4）~~ **done**（x0083）
5. **host RL / GLB budget / assets hermetic / WAN host E2E**（P1）を並行

スライス完了条件に必ず含める:

- **新しい保証は否定系 E2E で確認**
- **保証表とコードが一致**（過大主張を残さない）
- **新クレートは CI・cargo deny・テスト >0**（再発防止）
