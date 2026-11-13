# Alchemy — 改善提案書（まとめ / 2026-11-12）

> 根拠: [specific-weaknesses-2026-11-12.md](specific-weaknesses-2026-11-12.md)  
> 前回計画: [archive/2026-11-05/improvement-plan-2026-11-05.md](archive/2026-11-05/improvement-plan-2026-11-05.md)  
> 品質本線（物理）: [mvcc_xpbd.md](../../roadmap/mvcc_xpbd.md)（P1-2 / P1-3 / P3-2 を吸収；[レビュー取り込み](../../roadmap/mvcc_xpbd_review_proposal.md) 2026-11-13）  
> 規模本線（配信）: [spatial_interest.md](../../roadmap/spatial_interest.md)（**P3-7**；1000 人は horizon）  
> 目的: 採用マイナス ~-23 を潰し、正味 ~+109 をさらに押し上げる。  
> 更新: 2026-11-13 — MVCC×XPBD Phase 1 を「ゲート+scaffold 完了／残り作業あり」と明記。同日レビュー指摘で Phase 1 接触＝shared-sim のみ・Q1 採用済み・crash 正本規則・dirty reuse 契約を追記（文書反映 ≠ コード完了）。

---

## 方針

1. **見る権利・実行権・定義書き込み権を分離する**（HostDelegation + scene overlay）
2. **通常 UI ホストの資格情報寿命を閉じる**（親 client broker / IPC）
3. **権限・資源ゲートを先に閉じ、その後 MVCC×XPBD で物理品質本線を引き上げる**（接触・ゼロアロ・補間を対症療法で薄く直さない；等式距離のまま「接触オン」にしない）
4. **Room revoke と SceneBudget を門番に足す**
5. **handover / UI 一枚岩を衛生する**（計画的 handover は `sim_state`；差分配信は x0024＝直前 publish + 欠落時 snapshot）
6. **品質本線（MVCC×XPBD）と規模本線（Spatial Interest）を分離する** — フルメッシュのまま物理 Phase を増やして大規模 CCU を宣言しない
7. **構造変更は確定レーン**（inventory 一般化の `…/edit`）と物理ページを分離する — 設計は今、実装は Phase 3 並走可

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
| P1-1 | kick API + friendship → epoch | -2 | **closed**（x00a0） | Friends 解除後、旧鍵で新 epoch 平文が読めない否定系 |
| P1-2 | host で XPBD body-body 接触 | -2 | **対症療法の bounds 渡し／等式距離では閉じない**。[`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md): **Phase 1 = shared-sim ヘッドレス**（不等式接触＋箱形状）· **Phase 3 = host `bounds: Some` 配線** · [x00a3](../../roadmap/slices/x00a3-mvcc-phase1-scaffold.md) scaffold 済・能力は **残り** | Phase 1: `contact_never_attracts` + 2 箱積み閾値（host 本線は触らない）。Phase 3: host 接触オンでも地面・Grab 通過で本 ID を閉じる |
| P1-3 | hot-path zero-allocation | -2 | 現行 `EpochSheet` clone の薄修では閉じない。同上 Phase 1（真の CoW・`make_mut`・固定長履歴・スクラッチ再利用）に吸収 · x00a3 scaffold 済・alloc DoD は **残り**。計測手段（テスト専用クレート / `dhat`）は人承認（mvcc_xpbd §6 Q4） | Phase 1: tick 確保回数が上限以下（1k entity・500 Tick）+ 履歴窓内のページ再利用なし |
| P1-4 | handover 候補 accept 二段階 | -2 | **closed**（x00a1） | 非 accept 候補に lease が付かない否定系 |
| P1-5 | SceneBudget | -2 | **closed**（x00a2） | 上限超過 413/422 + 競合テスト |
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
| P3-1 | verifier Spot-Check orchestration | 目標 | Noop 配線のみ OSS。再シミュレーション要否は mvcc_xpbd §6 Q2（決まるまでクロスアーキ決定論は必須にしない） |
| P3-2 | Frame 補間 / jitter buffer | 提案 | **[`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) Phase 2 に吸収**（`sim_tick` 基準 + jitter・`wgpu` Compute・手元予測・ArcSwap 出版）。Phase 1 Green 後 |
| P3-3 | S2S 連合 / Spatial VCS 本体 | 目標 | Origin Cert v1 が素地。MVCC と独立 |
| P3-4 | billing / market / Relay QoS | 目標 | ディレクトリ未設置 |
| P3-5 | Formula / Wake / OSC / LOD / audio | 目標 | 減点しない。描画 LOD と **avatar sync LOD（規模本線）** を混同しない |
| P3-6 | 署名付き installer / 自動更新 | 提案 | Windows release 延長 |
| P3-7 | 配信の空間インデックス（Spatial Interest） | 規模本線 | 正本 [`spatial_interest.md`](../../roadmap/spatial_interest.md)。アウトラインは並走可；**実装は MVCC Phase 3 後**。roadmap README 旧メモ「P3-7 BLAKE3 ストリーミング」とは**無関係**（BLAKE3 ストリーミング禁止は継続） |

### MVCC×XPBD（品質本線・横断）

正本: [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md)（レビュー反映の詳細・未決 Q1–Q8 は同書 §6 / [review_proposal](../../roadmap/mvcc_xpbd_review_proposal.md)）。**大規模 CCU / 1000 人はここに書かない**（→ P3-7）。

**現状（2026-11-13）**: 着手ゲート P1-1/4/5 と scaffold（x00a3）まで。Phase 1 DoD（接触・ゼロアロ・真の CoW・レイアウト予約・クレート直下 `forbid`）は **文書反映済み・コード未着手**。

| Phase | 内容 | 吸収する ID | 着手条件 |
|:---|:---|:---|:---|
| Phase 1（残り） | `forbid` クレート直下・真の CoW・不等式接触（**shared-sim のみ**）・`frame_delta` 統一・回転/holder/`EntityDirectory` 予約・Android CI・alloc 上限 | P1-2（能力）/ P1-3 | **P1-1 / P1-4 / P1-5 closed**（済） |
| Phase 2 | MVCC バッファ + `wgpu` Compute + jitter（`sim_tick` 基準）+ 手元予測 + ArcSwap 出版 | P3-2 | Phase 1 のビット一致（同一アーキ）・接触回帰・alloc 上限・CoW が Green |
| Phase 3 | host 権威 + **host 接触オン（P1-2 完了）** + dirty 合成（x0024）+ `sim_state` handover（Q1 採用済み）+ 確定レーン並走 | P1-2（host 配線）+ host 本線置換 | Phase 2 Green。Room AEAD / wire_cap |

**やらないこと**: P1 ゲート前に Phase 2/3 を host/app へ配線する。現行 `xpbd.rs` に bounds だけ戻して／等式距離のまま P1-2 を閉じる。Phase 1 で host 接触オンだけして「P1-2 closed」と書く。**MVCC だけで大規模 CCU を宣言する**／フルメッシュのまま Phase を増やしてスケールしたことにする。描画用 frame の量子化値だけで「0 秒 handover 速度完全復元」を宣言する（→ `sim_state`）。

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
3. **kick/epoch + handover accept + SceneBudget**（P1-1 / P1-4 / P1-5）— **closed**（x00a0 / x00a1 / x00a2）
4. **MVCC×XPBD Phase 1 残り**（P1-2 能力 / P1-3）— scaffold 済（x00a3）；shared-sim 不等式接触・真の CoW・alloc・レイアウト・`forbid`・Android CI が**コード未着手**（host 接触オンは Phase 3）
5. **P1-6 / P2**（vault・UI 分割 / docs / CI）— Phase 1 と **並行可**
6. **MVCC×XPBD Phase 2**（P3-2 吸収）— jitter + 手元予測 + ArcSwap 出版 + `wgpu` Compute
7. **MVCC×XPBD Phase 3** — host 権威 + dirty デルタ（x0024）+ `sim_state` handover；確定レーン（`…/edit`）は並走可
8. **Spatial Interest 実装スライス群**（P3-7）— 正本 [`spatial_interest.md`](../../roadmap/spatial_interest.md) は先行済；**実装は Phase 3 Green 後**（採番は着手時。`x00a4` は予約しない）
9. **P3-1 / P3-3 以降** — MVCC / Interest と独立；品質本線の直後に必須ではない（Spot-Check 再シミュレーション要否は mvcc_xpbd §6 Q2）

スライス完了条件に必ず含める:

- **新しい保証は否定系 E2E で確認**
- **保証表とコードが一致**（特に権限表・物理非保証）
- **新クレートは CI・cargo deny・テスト >0**（再発防止）
- **MVCC 各 Phase は [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) の着手条件を満たしてから開始**
- **規模 CCU を宣言する変更は [`spatial_interest.md`](../../roadmap/spatial_interest.md) のゲートに従う**
