# Alchemy — マイナス点統合一覧（まとめ / 2026-10-30）

> 根拠: [Opus](opus/opus-specific-weaknesses-2026-10-30.md) · [GPT](gpt/gpt-specific-weaknesses-2026-10-30.md)  
> 採用ルール: 双方合意は折衷または厳しめ側、片方発見の実害は採用。前回まとめ（[archive/2026-10-28](archive/2026-10-28/specific-weaknesses-2026-10-28.md)）で **closed** と確認された項目は載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

| 採用合計（概算） | **-28** |

---

## P0 — 新公開門番（Relay / WAN / CAS policy）

### zenoh / relay-agent

- **WAN ACL の wildcard subscribe で `wire_cap` 部屋隔離が成立しない** `-3`（Opus -3 / GPT は ACL を partial）
  > guest/host ルールが `alchemy/dev/room/*/*/manifest` 等を許可する。同一ワイルドカードで subscribe できれば、guest 証明書 1 枚で全部屋のキーから `wire_cap` を読める。アプリ署名は残るが、private 部屋の内容・参加者 `sub` の機密性と put DoS 前提が崩れる。保証表の「`wire_cap` を知らない peer は他 room に到達できない」は過大。
  > 採用: **-3**（Opus。zenohd 実機否定系は未実行のため、まず保証表修正＋否定系で確認）
  > 対象ファイル: `zenoh-router/config.wan.json5`, `protocol/src/lib.rs`, `.workspace/0_doc/architecture/trust-boundary.md`

- **「300秒証明書」の実 NotAfter が 24 時間** `-3`（GPT -3 / Opus -1）
  > API は `expires_in=300` だが OpenSSL `-days 1`。漏えい鍵は最大 1 日 host/guest ACL を持つ。短命 mTLS の失効境界を満たさない。
  > 採用: **-3**（GPT）
  > 対象ファイル: `relay-agent/src/main.rs`

- **Allocation の資源上限・JWKS hard stale・body 上限不足** `-3`（GPT -3 / Opus 関連 -1〜-2）
  > soft-stale JWKS 無期限、無制限 `thread::spawn`、Content-Length body に 64KiB 未適用、`jti` 再利用制限なし。有効 Token 反復で鍵発行を飽和させられる。
  > 採用: **-3**（GPT）
  > 対象ファイル: `relay-agent/src/main.rs`

- **relay が秘密鍵を平文 HTTP で返し、手順書が `0.0.0.0` を指示** `-2`（Opus -2 / GPT 未個別）
  > サーバー側鍵生成 + `key_pem` JSON。Tunnel 背後でもなく、盗聴で Allocation Token と mTLS 鍵の両方が取れる。
  > 採用: **-2**（Opus）
  > 対象ファイル: `relay-agent/src/main.rs`, `.workspace/0_doc/manual/zenoh-wan-conoha.md`

### assets-server

- **CAS 物理オブジェクトの access 先着固定（policy squatting）** `-3`（GPT -3 / Opus は非保証として減点せず）
  > `CasObject.access` は hash ごとに一つ。先着 PUT が `authenticated` / `grant_required` を固定し、後続は 409。同一 bytes を公開 mesh と private inventory で併用できない。前回の「GET 無所有者検査」は文書化＋`grant_required` opt-in で **partial→置換**。
  > 採用: **-3**（GPT。GET 既定の authenticated 自体は非保証として継続非減点）
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

---

## P1 — 門番衛生・可用性・テスト穴

- **relay-agent が CI・`make ci`・cargo deny・単体テストから外れている** `-2`（Opus -2 / GPT -1）
  > セキュリティ境界なのにテスト 0・workflow path 外。前回 launcher と同型の再発。NotAfter/JWKS/body の契約違反を自動検出できない。
  > 採用: **-2**
  > 対象ファイル: `relay-agent/`, `.scripts/ci.sh`, `.github/workflows/rust-ci.yml`

- **adversarial E2E が実 client trust path を通らない** `-2`（GPT -2 / Opus -1）
  > host 生存は見るが、GUI/client の seal・gen/rev・discard、および WAN mTLS/ACL は未通。wildcard 問題を CI が見逃した直接原因。
  > 採用: **-2**
  > 対象ファイル: `client/e2e-probe/src/main.rs`, `docker-compose.e2e.yml`

- **world に rate limit / Room 作成上限が無い** `-2`（Opus -2 / GPT 未個別）
  > `POST /rooms`・Token 発行が無制限。Token 発行は auth S2S を増幅する。
  > 採用: **-2**
  > 対象ファイル: `world-server/lib/world_web/router.ex`

- **相関 ID / auth・world・relay メトリクス不足** `-1`（Opus -2 / GPT -1）
  > host/client/assets の loopback metrics は入った。横断追跡は未。
  > 採用: **-1**（折衷。metrics 本体 closed を認める）

- **launcher host restart 後の client IPC stale** `-1`（双方）
  > 採用: **-1**
  > 対象ファイル: `launcher/src/main.rs`

---

## P2 — 品質・DX・残債

- **README / client.md / vision 鮮度** `-1`（双方）
  > README は x004d / TLS / 次 ID x0053 のまま。実装は x004e〜x0052・次 ID x0066。
  > 採用: **-1**

- **軽微群（auth Keys 直列化、b3sum CLI、seal-hash CLI、reserved 回収なし、main/room_token 肥大、tos_agreed 固定、USER_ROOT 天井 4、DemoScene 手動同期、prod compose 0.0.0.0、cargo deny 範囲、mTLS 鍵 temp、腐った smoke 等）** `-1` 帯
  > 採用: まとめて **-3**

---

## スコア感（採用マイナス）

| 帯 | 点数 |
|:---|---:|
| P0 新門番（Relay/WAN/CAS） | ≈ -14 |
| P1 衛生・テスト・可用性 | ≈ -8 |
| P2 品質・DX | ≈ -6 |
| **合計概算** | **≈ -28** |

前回まとめ採用マイナス ≈ -48 から大幅減。残るのは「アプリ暗号契約は閉じた後の **transport 門番（relay）**」と **CAS access 配置**。
