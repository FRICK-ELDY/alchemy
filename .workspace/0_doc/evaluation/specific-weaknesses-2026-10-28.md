# Alchemy — マイナス点統合一覧（まとめ / 2026-10-28）

> 根拠: [Opus](opus/opus-specific-weaknesses-2026-10-28.md) · [GPT](gpt/gpt-specific-weaknesses-2026-10-28.md)  
> 採用ルール: 双方合意は折衷または厳しめ側、片方発見の実害は採用。前回まとめ（archive/2026-10-26）で解決済みと確認された項目は載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

| 採用合計（概算） | **-48** |

---

## P0 — 公開・暗号契約の穴（今すぐ）

### client / world — 署名まわり

- **偽 Manifest / Presence で全員強制退室** `-3`（Opus -3 / GPT は replay 側で別指摘）
  > client は pin / room / seal / attestation 失敗で `leave_room_local()` を呼ぶ。Zenoh ACL が無いため、到達 peer が空 Manifest を 1 パケット流すだけで部屋を空にできる。Frame の「破棄」と一貫していない。
  > 採用: **-3**（Opus）
  > 対象ファイル: `client/app/src/main.rs`

- **ManifestSeal が任意 body hash の署名 oracle** `-4`（GPT -4 / Opus 未個別）
  > world は永続 scene を持つが、seal は owner host が送った任意 revision/hash を署名するだけで DB scene と照合しない。Untrusted host が任意 entity を「world 正規 seal」付きで配信できる。
  > 採用: **-4**（GPT）
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/manifest_seal.ex`

- **署名済み Manifest / Presence の revision replay 不在** `-4`（GPT -4 / Opus 未個別）
  > 有効期限内の古い署名 state を反復適用でき、旧 host key への巻き戻しで現 Frame を拒否させられる。
  > 採用: **-4**（GPT）
  > 対象ファイル: `client/app/src/main.rs`

- **Zenoh protobuf に decode byte/count 上限なし** `-4`（GPT -4 / Opus 未個別・関連で ACL -1）
  > 署名検証より先に decode・allocation が走り、到達 peer から memory/CPU DoS が可能。
  > 採用: **-3**（折衷。実害は明確だがルーター側制限と合わせて段階対応）
  > 対象ファイル: `protocol/src/lib.rs`, `client/host-server/src/main.rs`, `client/app/src/main.rs`

### 運用・配布

- **prod compose が `AUTH_INTERNAL_TOKEN` を渡さず起動しない** `-3`（Opus -3 / GPT 未個別）
  > x0035 で prod fail-closed になった必須 env が compose / `.env.prod.example` に無く、手順書どおりでは auth/world が raise で落ちる。
  > 採用: **-3**（Opus）
  > 対象ファイル: `auth-server/docker-compose.prod.yml`, `world-server/docker-compose.prod.yml`

---

## P1 — 門番・可用性・CAS

### assets-server

- **認証済み全員が hash だけで他人 Blob を GET できる** `-4`（GPT -4 / Opus 前回折衷継続・今回個別弱）
  > owner_sub は GET 認可に使われない。private/paid UGC の confidentiality を保証しない。
  > 採用: **-3**（前回同。capability モデル未実装のまま）
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`

- **CAS object と owner reference の混同・補償なし** `-2`（GPT -2 / Opus は TX 内 put として -1）
  > cross-owner dedupe で新 owner の ref/quota が作られず、put 後 DB 失敗の orphan もある。
  > 採用: **-2**
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### host-server

- **物理ループ内の同期 HTTP / Zenoh wait + 1 回失敗でプロセス終了** `-2`（双方 -2）
  > 最悪約 20 秒停止、auth 一時障害で部屋消失。fail-closed は「失効で止める」であり「失敗で殺す」ではない。
  > 採用: **-2**
  > 対象ファイル: `client/host-server/src/main.rs`

- **bind 時 JWT 盗聴でセッション乗っ取り（cnf 無し・後勝ち）** `-2`（Opus -2 / GPT 未個別）
  > 定常 Intent から JWT は外れたが bind はワイヤに載る。
  > 採用: **-2**
  > 対象ファイル: `client/host-server/src/session.rs`

- **per-sub Intent rate limit / JWKS hard stale expiry 不在** `-2`（GPT 各 -2 / Opus 部分言及）
  > 合法高頻度 Intent の飽和、鍵撤去後の無限 stale 受理。
  > 採用: **-2**（まとめて 1 項）
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/room_token.rs`

### zenoh / edge

- **WAN TLS のみで mTLS / namespace ACL / QoS 不在** `-3`（GPT -3 / Opus -1）
  > 盗聴は防いだが publisher identity 隔離はない。上記 DoS・leave・replay の前提条件。
  > 採用: **-2**（折衷。暗号契約を先に閉じ、ACL は並行）
  > 対象ファイル: `zenoh-router/config.wan.json5`

---

## P2 — 品質・DX・残債

### テスト

- **world ローカル CI が stale / 共有 DB で汚染** `-2`（双方 -2）
  > assets で直した型の再発。`make ci` が手元で赤になる。
  > 採用: **-2**
  > 対象ファイル: `world-server/config/runtime.exs`, `world-server/config/test.exs`

- **E2E が正常系中心・adversarial / 実 client trust 未通** `-2`（双方）
  > 偽 Manifest leave・replay・oversize は CI で見つかっていない。
  > 採用: **-2**
  > 対象ファイル: `client/e2e-probe/src/main.rs`, `.github/workflows/e2e-compose.yml`

### client / launcher / 横断

- **asset 失敗を Coin fixture で隠す / HTTP timeout 方針なし** `-2`（GPT）
  > 採用: **-2**
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/world_api.rs`

- **描画経路二重化（vk_loop 肥大）** `-2`（Opus 継続）
  > 採用: **-2**
  > 対象ファイル: `client/app/src/surface/xr/vk_loop.rs`

- **メトリクス / OTLP 不在** `-2`（双方）
  > 採用: **-2**

- **launcher CI 外・clippy 失敗・host restart 後 IPC stale** `-1`（双方）
  > 採用: **-1**
  > 対象ファイル: `launcher/src/main.rs`

- **README / roadmap / vision 鮮度、ClientIp 複製、inspect(reason)、Grab aim 上限、USER_ROOT_SLOTS の protocol 固定 等** `-1` 帯
  > 採用: まとめて **-3**（軽微群）

---

## スコア感（採用マイナス）

| 帯 | 点数 |
|:---|---:|
| P0 暗号・公開 | ≈ -17 |
| P1 門番・可用性 | ≈ -13 |
| P2 品質・DX | ≈ -18 |
| **合計概算** | **≈ -48** |

前回まとめ採用マイナス ≈ -72 から大幅減。残るのは「署名がある＝定義を保証した」までの未完と、継ぎ目（prod env・leave on fail・test DB）。
