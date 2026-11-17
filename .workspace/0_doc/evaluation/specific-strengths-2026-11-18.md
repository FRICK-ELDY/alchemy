# Alchemy — プラス点統合一覧（まとめ / 2026-11-18）

> 根拠: [Opus](opus/opus-specific-strengths-2026-11-18.md) · [GPT](gpt/gpt-specific-strengths-2026-11-18.md)  
> 採用ルール: 双方が検証できた境界をコアに採用。Opus の細分再加点は主指標にせず、今回閉じた契約と新規境界を優先する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 業界の一般的なベストプラクティスに沿う |
| +3 | 同規模平均を明確に上回る |
| +4 | プロダクション級 OSS と遜色ない |
| +5 | 個人プロジェクトとして異例の卓越 |

| 採用合計（概算） | **~+150** |

---

## 運営側コア

### auth-server

- **アイデンティティ SSoT・session lifecycle・多軸 rate limit** `+5`（双方）
  > Argon2、RS256/kid/JWKS、refresh family、列挙抑止、IP/user/family 軸 RL。Room/CAS 権限を JWT に混入させない。
  > 対象ファイル: `auth-server/lib/auth/`, `auth-server/lib/auth_web/`

- **relation_changed の S2S フック（本番 URL 未設定は起動 fail-closed）** `+1`（双方・深さは弱点側）
  > 発火・推移・耐久の穴は弱点で計上。フック自体の導入は前進。
  > 対象ファイル: `auth-server/lib/auth/world_notify.ex`

### assets-server

- **CAS・Quota・Grant の台帳分離** `+5`（双方）
  > 物理 bytes / 容量 / 読取権を分離。予約→TX 外 put→確定、orphan quarantine。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/`

- **Spatial VCS accept path（決定的 CBOR・Port 再検証・ref CAS・Origin Cert v2）** `+4`（双方）
  > git receive-pack 相当を Alchemy 原則で再設計。連合の下地。
  > 対象ファイル: `assets-server/lib/assets/vcs/`, `protocol/src/vcs_object.rs`

- **SpawnTicket（Grant → 短命証票 → world 一回消費）** `+3`（双方）
  > host を信頼せず配置権を移送。層分離の体現。
  > 対象ファイル: `assets-server/lib/assets/spawn_ticket.ex`, `world-server/lib/world/spawn_ticket.ex`

### world-server

- **見る権利・実行権・定義書込の分離（HostDelegation）** `+5`（双方・前回 P0 閉鎖）
  > canonical = owner∪editor、takeover host = overlay のみ、eligible allow-list、handover accept 二段階、lease CAS。否定系あり。
  > 対象ファイル: `world-server/lib/world/rooms/lease.ex`, `world-server/lib/world/rooms/scene.ex`

- **PoP nonce・Room key epoch・packed Scene CAS・SceneBudget** `+4`（双方）
  > bearer 単独・skew 内 replay・構造肥大を別層で閉じる。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/rooms/`

- **Rooms / Fallback 分割と権限否定系の厚み** `+3`（Opus 強・GPT 合意）
  > 一枚岩解消。権限表の見落とし耐性が上がった。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/fallback_controller.ex`

---

## 共通契約・シミュレーション

### protocol

- **ワイヤ・暗号・PIN・decode ceiling の SSoT** `+5`（双方）
  > AEAD、session 署名、generation gate、edit/sim_state/interest 語彙を独立 crate に。
  > 対象ファイル: `protocol/proto/`, `protocol/src/`

### shared-sim / host-server

- **MVCC page・決定論 SIMD・不等式接触・edit レーン** `+4`（双方）
  > page-alloc 上限・replay 一致は実証。edit は封筒上限→Dual-auth→成功後 Commit。
  > 対象ファイル: `client/shared-sim/src/mvcc/`, `client/host-server/src/edit.rs`

- **Untrusted input の多層 gate・Spatial Interest host 側** `+4`（双方）
  > JWT/cnf・sequence・finite・rate・Leave/TTL。Room frame fail-closed / cell soft-fail の方針分離。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/network_worker.rs`

---

## ホスト端末・転送

### client/app + HostSupervisor

- **親 client 資格情報 broker（AUTH_PUSH）+ OS vault** `+5`（双方・前回 P0 閉鎖）
  > refresh は親のみ。子へは短命 access JWT。prefs は key id のみ。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/app/src/refresh_vault.rs`

- **KeyStale 無停止切替・Desktop イベント→コマンド分割・手元予測/jitter** `+3`（双方）
  > P2 衛生と体感土台。
  > 対象ファイル: `client/app/src/surface/desktop/`, `client/app/src/sim_jitter.rs`

### relay / edge / relay-agent

- **CSR-only・ureq JWKS・新キー明示 ACL（frame/\*\* 非拡大）** `+3`（双方）
  > 前回残件を閉じ、Spatial Interest キーを安全に追加。
  > 対象ファイル: `relay-agent/`, `relay-server/config.wan.json5`

---

## 横断

- **評価駆動の自己改善（前回 P0 を権限モデルごと閉鎖）** `+4`（双方）
  > 継ぎ当てではなく定義↔実行を直した。1 週間 58 スライス。
- **テスト密度の伸長（client 708 / world 232 / assets 240 / protocol 124）** `+3`（双方）
  > 権限否定系が厚い。E2E 追随は弱点側。
- **非保証欄の継続運用・依存承認日** `+2`（Opus）
  > 弱点の多くが非保証欄から発見可能。

---

## 採用プラスの扱い

Opus +467 は継続項目の細分再加点を含む。GPT +89 は検証境界に絞る。まとめは **合意コア + 今回閉じた契約 + 新規境界** を採用し、主指標は「旧 P0 閉鎖と新穴」とする。概算 **~+150**（前回まとめ採用 ~+132）。
