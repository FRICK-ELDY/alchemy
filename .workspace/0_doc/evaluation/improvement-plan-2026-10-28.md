# Alchemy — 改善提案書（まとめ / 2026-10-28）

> 根拠: [specific-weaknesses-2026-10-28.md](specific-weaknesses-2026-10-28.md)  
> 前回計画: [archive/2026-10-26/improvement-plan-2026-10-26.md](archive/2026-10-26/improvement-plan-2026-10-26.md)  
> 目的: 採用マイナスを優先度付きで潰し、正味 ~+44 をさらに押し上げる。

---

## 方針

1. **暗号契約の中身を閉じる**（leave→discard / seal oracle / replay / decode limit）— 署名の「ある」から「意味がある」へ
2. **運用継ぎ目を直す**（prod compose S2S / world test DB）— 手順書どおりに起動し、手元 CI を緑に
3. **CAS 門番と可用性**（GET 認可 / physics-network 分離）
4. **transport ACL**（mTLS・namespace）はアプリ契約の後段でも可
5. **可観測性・adversarial E2E・描画統合**は並行負債

前回 P0〜P3 の大半はコードで閉じた。残課題と新規をこの版に載せ替える。

---

## P0 — 今すぐ（暗号契約・公開運用）

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | 検証失敗は破棄（leave しない） | -3 | Manifest/Presence の pin・seal・attest 失敗は drop+counter。退室は検証済み世代切替のみ。trust-boundary.md 更新 | 偽 Manifest 1 パケットで部屋が空にならない。否定系 E2E 緑 |
| P0-2 | ManifestSeal を canonical に | -4 | world が DB scene から Manifest body/hash を構築し署名。host 指定 hash は照合必須または廃止 | owner host が任意 hash で正規 seal を取れないテスト |
| P0-3 | generation + revision 単調ゲート | -4 | ManifestSeal/Presence に host_generation・monotonic revision。client は後退拒否。handoff のみ新 generation | 古い署名 Manifest の replay で Frame が止まらない — **x003a done** |
| P0-4 | decode byte/count limit | -3 | Zenoh sample 受信時に class 別上限。decode 後 entries/strings 上限。巨大 payload テスト | 巨大 protobuf で OOM しない — **x003b done** |
| P0-5 | prod compose に S2S env | -3 | `AUTH_INTERNAL_TOKEN` / `AUTH_INTERNAL_URL` を compose と `.env.prod.example` に。CI で `compose config` + `/health` | 手順書どおり prod が起動する — **x003b done** |
| P0-6 | world テスト DB 隔離 | -2 | test で `WORLD_DATABASE_PATH` 無視 or 一時 path。ci.sh 強制。Sandbox 後始末 | 日常スタック後も `make ci` world 緑 — **x003b done** |
| P0-7 | ZIP/launcher: Coin は assets DL 必須 | ZIP 煙 | Manifest blake3 → assets GET 成功。fixture は seed ソースのみ | ZIP Start で fixture fallback 無し（release）で mesh 表示 — **[x003c](../../roadmap/slices/x003c-assets-required-mesh.md) done** |

---

## P1 — 門番・可用性・bind

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | assets GET 方針の文書化 **または** BlobGrant | -3 | 短期: architecture に capability モデル明記。中期: CasObject/BlobRef/BlobGrant | 保証表とコード一致 — **文書化で閉じた**（[assets-server.md](../architecture/assets-server.md) GET capability）。BlobGrant 実装は [assets-server-draft.md](../architecture/assets-server-draft.md) / P4-5 |
| P1-2 | CAS object/ref 分離 + put 補償 | -2 | Quota 予約→TX 外 put→メタ確定。cross-owner ref | orphan/quota 不整合テスト — **[x003f](../../roadmap/slices/x003f-cas-object-ref-reserve.md) done** |
| P1-3 | credential/network worker 分離 | -2 | 物理は最新 cred を読むだけ。更新は専用スレッドで失効までリトライ。プロセスは一時 502 で殺さない | auth 一時障害で部屋が消えない — **[x003d](../../roadmap/slices/x003d-credential-worker.md)** credential + **[x0042](../../roadmap/slices/x0042-zenoh-network-worker.md)** Zenoh put **done** |
| P1-4 | Room Token `cnf`（session_pub） | -2 | Token 要求時に鍵を送り claim に埋め込み。bind で一致確認 | 盗聴 JWT だけでは別鍵 bind 不可 — **[x003e](../../roadmap/slices/x003e-room-token-cnf.md) done** |
| P1-5 | Intent rate limit + JWKS hard stale | -2 | per-sub token bucket。JWKS 最大 stale TTL | 合法高頻度で飽和しない / 撤去鍵で無限受理しない — **[x0040](../../roadmap/slices/x0040-intent-rate-jwks-hard-stale.md) done** |
| P1-6 | Grab 保持中 aim 距離クランプ | -1 | shared-sim 定数を保持オフセットにも適用 | 箱の瞬間遠方移動不可 — **[x0041](../../roadmap/slices/x0041-grab-hold-aim-clamp.md) done** |

---

## P2 — ワイヤ身元・エッジ

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P2-1 | Zenoh mTLS + room/tenant ACL | -2 | キー族 publish 権分離。allocation token | 任意 peer が他 room の manifest に書けない — **[x004e](../../roadmap/slices/x004e-relay-agent-mvp.md) done** |
| P2-2 | `/internal/*` ingress 404 + Social RL | -1 | `/dev/*` と同型。Follow API に rate_limited | 公網から internal に届かない — **[x0043](../../roadmap/slices/x0043-internal-ingress-social-rl.md) done** |
| P2-3 | client world_jwks クールダウン | -1 | host と同型 SWR + 未知 kid 冷却 | 偽 kid 連打で受信スレッドが詰まらない — **[x0044](../../roadmap/slices/x0044-client-world-jwks-cooldown.md) done** |

---

## P3 — 品質基盤・負債

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P3-1 | adversarial E2E（偽 Manifest / stale-rev / oversize） | -2 | e2e-probe + compose fault — **[x0046](../../roadmap/slices/x0046-adversarial-e2e.md) done** |
| P3-2 | launcher を CI に・clippy 修正・再起動ポリシー単体テスト | -1 | workflow + `make ci-launcher` — **[x0047](../../roadmap/slices/x0047-launcher-ci-restart-policy.md) done** |
| P3-3 | fixture fallback を debug 限定 + HTTP timeout/cancel | -2 | placeholder UI — **[x0048](../../roadmap/slices/x0048-asset-placeholder-http-timeout.md) done** |
| P3-4 | metrics（tick lateness / frame age / JWKS stale / CAS） | -2 | localhost Prometheus text（依存なし）— **[x0049](../../roadmap/slices/x0049-minimal-metrics.md) done** |
| P3-5 | README / roadmap / vision 鮮度 | -1 | 現状を x0049 まで反映 — **[x004a](../../roadmap/slices/x004a-docs-freshness.md) done** |
| P3-6 | ClientIp 共有化・`inspect(reason)` 除去 | -1 | alchemy_auth_client 拡張 — **[x004b](../../roadmap/slices/x004b-client-ip-shared-inspect.md) done** |
| P3-7 | BLAKE3 ストリーミング・Room 作成上限 | -1 | NIF / rate limit — **マージしない**（実装試行あり・**CI が完了しなくなる**ため） |
| P3-8 | host Room Token 否定系テスト | -1 | alg/iss/aud/exp/room — **[x0045](../../roadmap/slices/x0045-room-token-negative-tests.md) done** |
| P3-9 | cargo-fuzz（protobuf/JWT） | 提案 | protocol / host — **[x004c](../../roadmap/slices/x004c-cargo-fuzz-protobuf-jwt.md) done** |
| P3-10 | USER_ROOT_SLOTS を Manifest へ | -1 | protocol は帯規則のみ — **[x004d](../../roadmap/slices/x004d-user-root-slots-manifest.md) done** |

---

## P4 — アーキテクチャ（スライス推奨）

| ID | 項目 | 根拠 | 備考 |
|:---|:---|:---|:---|
| P4-1 | 描画経路統合（wgpu + OpenXR） | -2 | vk_loop 肥大の根本 — **[x0050](../../roadmap/slices/x0050-xr-swapchain-wgpu.md)**/**[x0051](../../roadmap/slices/x0051-xr-scene-gpu-unify.md)** done → **[x0052](../../roadmap/slices/x0052-xr-vk-loop-gpu-remove.md)**（planned） |
| P4-2 | launcher discovery / deep link / handoff | 提案 | IPC stale 解消を含む |
| P4-3 | verifier（Spot-Check） | 目標 | **P0-2/P0-3 が先行条件** |
| P4-4 | Formula VM / Wake-on-Join / OSC / LOD / audio | 目標 | 減点しない |
| P4-5 | billing / market / BlobGrant 経済圏 | 目標 | **台帳 MVP（案 A）= [x004f](../../roadmap/slices/x004f-blob-grant-mvp.md)**。GET 常時 Grant（案 B）・Vault / market は後続 |

---

## 前回計画のクローズ状況（要約）

| 前回帯 | 状態 |
|:---|:---|
| P0 Tunnel / host_control / seed / assets CI | **ほぼ完了** |
| P1 Token refresh / JWKS SWR / magnitude / host role / GET 文書 | refresh・JWKS・role・clamp **完了**。GET capability 文書も **完了**（BlobGrant 実装は未・P4-5） |
| P2 liveliness / Token バス / WAN TLS / Frame 検証 / PIN | **ほぼ完了**（bind JWT・ACL 残） |
| P3 衛生一式 | **大半完了**（world test DB は同型再発） |
| P4 Manifest 駆動 / shared-sim / launcher | **最小完了**。canonical seal・generation は今回 P0 へ昇格 |

---

## 推奨スライス順

**次 / 進行中:** **[x004f](../../roadmap/slices/x004f-blob-grant-mvp.md)** BlobGrant MVP（案 A）。目標形は案 B（GET の SSoT = Grant + room 連動）。  
**P3-7**（BLAKE3 ストリーミング・Room 作成上限）は試行したが **CI が完了しなくなるためマージしない**（再開時は CI 完走を先行条件に）。

最近閉じた: **[x004e](../../roadmap/slices/x004e-relay-agent-mvp.md)**（P2-1 Relay Agent MVP）、**[x004d](../../roadmap/slices/x004d-user-root-slots-manifest.md)**（P3-10）。

スライス完了条件に必ず含める: **prod compose 起動**・**否定系が通る**・**保証表とコード一致**。
