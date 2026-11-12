# Alchemy — マイナス点統合一覧（まとめ / 2026-11-12）

> 根拠: [Opus](opus/opus-specific-weaknesses-2026-11-12.md) · [GPT](gpt/gpt-specific-weaknesses-2026-11-12.md)  
> 採用ルール: 双方合意は折衷または厳しめ側、片方発見の実害は採用。前回まとめ（[archive/2026-11-05](archive/2026-11-05/specific-weaknesses-2026-11-05.md)）で **closed** と確認された項目は載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

| 採用合計（概算） | **-23** |

---

## P0 — 新権限境界（takeover・host 資格情報）

### world / HostSupervisor

- **可視参加者の無委譲 takeover が、canonical scene 改変権まで渡す** `-4`（GPT -4 / Opus -3）
  > dormant の public Room は、入室 ACL だけで非オーナーが `active_host` になれる（controller test も成功条件）。その後 host Token + PoP で scene append/delete / seal / attestation を正規実行できる。「見る権利」と「定義を書く権利」が混在し、server=定義 / host=実行の原則に反する。
  > 採用: **-4**（GPT 厳しめ。Opus の定義↔実行逆転指摘を包含）
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/rooms.ex`, `world-server/test/world_web/controllers/room_controller_test.exs`

- **通常 UI ログインで起動した host が、約 15 分で資格情報更新不能になる** `-3`（GPT -3 / Opus -2）
  > HostSupervisor は access JWT のみを子へ渡し、refresh を渡さない。host は `expires_in=0` 扱いのため auth refresh せず、access TTL 後に Room Token 再発行が失敗し配信停止。`AUTH_USER`/password 回避は本線ではない。
  > 採用: **-3**（GPT。配布本線の欠陥として）
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/host-server/src/room_token/credential_chain.rs`

---

## P1 — 移譲衛生・物理後退・資源上限

- **Room 鍵 epoch は入ったが、kick / friendship 解除の即時 revoke が未完** `-2`（GPT -2 / Opus -1）
  > visibility 縮小・定期・owner 手動は実装済み。HTTP kick ルート無し、friendship 変更では回転しない。旧参加者は最長約 24h 復号可能。
  > 採用: **-2**
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/router.ex`

- **Rapier → XPBD 置換で body-body 接触が host で切られた** `-2`（Opus -2）
  > host `step_dt` は bounds なし・「LBVH body-body intentionally off」。箱同士すり抜け。未使用 `rapier3d` 依存と overview の Rapier 表記も残る。
  > 採用: **-2**
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/Cargo.toml`

- **shared-sim / host ホットパスが 60Hz で全 sheet clone と一時確保を回す** `-2`（GPT -3 / Opus -1）
  > tick あたり複数 `EpochSheet::clone`、彩色 HashSet、LBVH leaf 線形探索で構築が O(n²) 寄り。DOD / ゼロアロケーション規則と不一致。
  > 採用: **-2**（折衷。規模保証未達として）
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/shared-sim/src/xpbd.rs`, `client/shared-sim/src/spatial.rs`

- **Graceful handover が opt-in を見ず Presence 先頭を 5 分指名する** `-2`（GPT -2）
  > 候補が起動しなければ他候補 takeover を lease 失効まで塞ぐ。
  > 採用: **-2**
  > 対象ファイル: `client/app/src/scene.rs`, `client/app/src/surface/desktop.rs`, `world-server/lib/world/rooms.ex`

- **canonical scene の構造上限がない** `-2`（GPT -2）
  > entries / JSON bytes 上限なし。host RL は時間窓だけで長期肥大を止めない。
  > 採用: **-2**
  > 対象ファイル: `world-server/lib/world/rooms.ex`

- **7 日 refresh token を平文 prefs JSON に保存** `-1`（GPT -2 → 採用圧縮）
  > OS vault / 0600 強制なし。同一端末脅威モデルでは弱いが、ログ漏洩ではない。
  > 採用: **-1**
  > 対象ファイル: `client/app/src/operator_prefs.rs`

---

## P2 — 品質・DX・残債

- **`dash.rs` / `desktop.rs` の肥大が倍増** `-2`（Opus -2 open 悪化）
  > 3604 / 3805 行。Facet 分離とテスト容易性を阻害。
  > 採用: **-2**
  > 対象ファイル: `client/app/src/system_ui/dash.rs`, `client/app/src/surface/desktop.rs`

- **world 軽微群（catalog RL 無し・scene append が Grant 非照会・PoP skew 内リプレイ・epoch 15s 全員ポーリング・rooms 一枚岩）** `-1`（Opus -2/-1 群を圧縮）
  > 採用: まとめて **-1**
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`, `world-server/lib/world/rooms.ex`, `world-server/lib/world/http_pop.ex`

- **GLB 画像 pixel 判定がフルデコード後 / KeyStale 手動 Rejoin（XR 非保証） / docs・release 残骸 / assets credo フレーク / relay-agent curl+expect** `-1` 帯（双方・採用合計 **-2**）
  > P1-2 画像順は partial。定期 epoch でも Desktop 手動 Rejoin。overview 等に Rapier / launcher 残骸。assets CI 初回 credo クラッシュは再実行 PASS。
  > 採用: まとめて **-2**

---

## スコア感（採用マイナス）

| 帯 | 点数 |
|:---|---:|
| P0 新権限境界 | ≈ -7 |
| P1 移譲・物理・資源 | ≈ -11 |
| P2 品質・DX | ≈ -5 |
| **合計概算** | **≈ -23** |

前回まとめ採用マイナス ≈ -24 から微減。旧 P0（PoP / scene CAS / key_pem / host RL / GLB budget 等）はほぼ閉じ、穴は **takeover 権限表・UI host 資格情報寿命・XPBD 後退** に移った。
