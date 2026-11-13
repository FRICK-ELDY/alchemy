# Alchemy — 改善提案書（まとめ / 2026-11-12）

> 根拠: [specific-weaknesses-2026-11-12.md](specific-weaknesses-2026-11-12.md)  
> 前回計画: [archive/2026-11-05/improvement-plan-2026-11-05.md](archive/2026-11-05/improvement-plan-2026-11-05.md)  
> 物理本線: [mvcc_xpbd.md](../../roadmap/mvcc_xpbd.md)（P1-2 / P1-3 / P3-2 を吸収）  
> 目的: 採用マイナス ~-23 を潰し、正味 ~+109 をさらに押し上げる。

---

## 方針

1. **見る権利・実行権・定義書き込み権を分離する**（HostDelegation + scene overlay）
2. **通常 UI ホストの資格情報寿命を閉じる**（親 client broker / IPC）
3. **権限・資源ゲートを先に閉じ、その後 MVCC×XPBD で物理本線を引き上げる**（接触・ゼロアロ・補間を対症療法で薄く直さない）
4. **Room revoke と SceneBudget を門番に足す**
5. **handover / UI 一枚岩を衛生する**

前回 P0（PoP / scene CAS / CSR-only / host RL / GLB budget / hermetic / WAN host E2E / Allocation throughput）と P2 大半はコードで閉じた。残課題と新規をこの版に載せ替える。

---

## P0 — 今すぐ（権限・資格情報）

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | HostDelegation + canonical scene を owner/editor 限定 | -4 | **closed**（x009d + x009e / #189–#190） | 非オーナー canonical 403；dormant で overlay 消失；非 eligible takeover 403 |
| P0-2 | 親 client 資格情報 broker | -3 | **closed**（x009f）: HostSupervisor が refresh を子に渡さず、loopback IPC `AUTH_PUSH` で更新済み access JWT を push。ParentBroker は自己 login しない | 偽 IPC トークン拒否；`apply_parent_auth_push` + IPC roundtrip；親 refresh→push 配線 |

---

## P1 — 移譲衛生・物理・資源

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | kick API + friendship → epoch | -2 | `POST /rooms/:id/room_key/rotate`（owner・RL）。auth relation 変更で CAS ローテ。**MVCC Phase 1 の前提ゲート** | Friends 解除後、旧鍵で新 epoch 平文が読めない否定系 |
| P1-2 | host で XPBD body-body 接触 | -2 | **対症療法の bounds 渡しのみでは閉じない**。[`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) Phase 1（既存 `shared-sim` 進化）の DoD に吸収 | Phase 1: 2 箱が重ならない回帰 + ページ／アリーナ側接触保証 |
| P1-3 | hot-path zero-allocation | -2 | 現行 `EpochSheet` clone の薄修では閉じない。同上 Phase 1（`HotPhysicsPage` / プール常駐）に吸収 | Phase 1: tick 確保回数が上限以下（1k entity・500 Tick） |
| P1-4 | handover 候補 accept 二段階 | -2 | Presence 先頭指名をやめ、候補 accept 後に CAS。不在時は即 dormant。**MVCC Phase 1 の前提ゲート** | 非 accept 候補に lease が付かない否定系 |
| P1-5 | SceneBudget | -2 | entries / JSON bytes を protocol decode 上限以下で原子検査。**MVCC Phase 1 の前提ゲート** | 上限超過 413/422 + 競合テスト |
| P1-6 | refresh token を OS vault へ | -1 | prefs は key id のみ。不能時は session-only。MVCC と直交（並行可） | 平文 refresh がディスクに残らない |

---

## P2 — 負債・衛生

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P2-1 | dash / desktop イベント→コマンド分割 | -2 | session/inventory/hotswitch flow を UI から分離 |
| P2-2 | catalog RL / scene Grant 照会 / PoP nonce / epoch 軽量 API | -1 | world 軽微群を 1〜2 スライスで |
| P2-3 | GLB 画像は寸法先読みしてから decode | -1 | `into_dimensions` → budget → decode |
| P2-4 | KeyStale 無停止二相切替 + XR Rejoin | -1 | grace 中に bus generation 原子切替 |
| P2-5 | docs / release の Rapier・launcher 残骸除去 | -1 | overview / client.md / protocol.md / README / windows-release.yml |
| P2-6 | assets CI 用 `_build` ボリューム分離 | -1 | 開発コンテナと共有しない |
| P2-7 | relay-agent JWKS を HTTP クライアント化（依存承認後） | -1 | curl 子プロセスと lock `expect` を解消 |
| P2-8 | Rooms / FallbackController 分割 | -1 | エラー表一箇所化 |

---

## P3 — アーキテクチャ（スライス推奨）

| ID | 項目 | 根拠 | 備考 |
|:---|:---|:---|:---|
| P3-1 | verifier Spot-Check orchestration | 目標 | Noop 配線のみ OSS。MVCC と独立 |
| P3-2 | Frame 補間 / jitter buffer | 提案 | **[`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) Phase 2 に吸収**（`wgpu` Compute・既存 `epoch_mirror` / Prev-Curr の段階置換）。Phase 1 Green 後 |
| P3-3 | S2S 連合 / Spatial VCS 本体 | 目標 | Origin Cert v1 が素地。MVCC と独立 |
| P3-4 | billing / market / Relay QoS | 目標 | ディレクトリ未設置 |
| P3-5 | Formula / Wake / OSC / LOD / audio | 目標 | 減点しない |
| P3-6 | 署名付き installer / 自動更新 | 提案 | Windows release 延長 |

### MVCC×XPBD（物理本線・横断）

正本: [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md)。improvement-plan 上の差し込み位置:

| Phase | 内容 | 吸収する ID | 着手条件 |
|:---|:---|:---|:---|
| Phase 1 | 既存 `client/shared-sim` を `HotPhysicsPage` / `MvccPageArena` / 決定論 XPBD へ進化（クレート新設ではない） | P1-2 / P1-3 | **P1-1 / P1-4 / P1-5 が否定系付きで closed** |
| Phase 2 | `client/app` の MVCC バッファ + `wgpu` Compute 補間 | P3-2 | Phase 1 のビット一致リプレイ・接触回帰・alloc 上限が Green |
| Phase 3 | `host-server` 権威ループ + XOR デルタ（Room AEAD / wire_cap） | （host 本線置換） | Phase 2 のシェーダ検証・描画経路が Green |

**やらないこと**: P1 ゲート前に Phase 2/3 を host/app へ配線する。現行 `xpbd.rs` に bounds だけ戻して P1-2 を閉じ、MVCC を無期限 P3 に棚上げする。

---

## 前回計画のクローズ状況（要約）

| 前回帯 | 状態 |
|:---|:---|
| P0-1 制御面 + PoP | **完了**（x0081 + x0098） |
| P0-2 Room key epoch | **部分完了** — 実装済みだが kick/friendship は今回 P1-1 |
| P0-3 Scene CAS | **完了**（x0082） |
| P0-4 Agent CSR-only | **完了**（x0083） |
| P1-1〜P1-5 | **完了**（host RL / GLB / hermetic / WAN host E2E / Allocation）。GLB 画像順のみ残 → P2-3 |
| P2-1〜P2-14 | **大半完了**。dash/desktop と docs 再ずれが残 |

---

## 推奨スライス順

1. **HostDelegation + scene overlay**（P0-1）— **closed**（x009d / x009e）
2. **資格情報 broker**（P0-2 / x009f）— **closed**
3. **kick/epoch + handover accept + SceneBudget**（P1-1 / P1-4 / P1-5）— **MVCC 着手前の必須ゲート**
4. **MVCC×XPBD Phase 1**（P1-2 / P1-3 吸収）— 既存 `shared-sim` 進化；接触 + ゼロアロを DoD に含む
5. **P1-6 / P2**（vault・UI 分割 / docs / CI）— Phase 1 と **並行可**
6. **MVCC×XPBD Phase 2**（P3-2 吸収）— `wgpu` Compute 補間
7. **MVCC×XPBD Phase 3** — host 権威ループ + XOR デルタ配信
8. **P3-1 / P3-3 以降** — MVCC と独立；物理本線の直後に必須ではない

スライス完了条件に必ず含める:

- **新しい保証は否定系 E2E で確認**
- **保証表とコードが一致**（特に権限表・物理非保証）
- **新クレートは CI・cargo deny・テスト >0**（再発防止）
- **MVCC 各 Phase は [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) の着手条件を満たしてから開始**
