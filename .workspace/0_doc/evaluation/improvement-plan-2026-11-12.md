# Alchemy — 改善提案書（まとめ / 2026-11-12）

> 根拠: [specific-weaknesses-2026-11-12.md](specific-weaknesses-2026-11-12.md)  
> 前回計画: [archive/2026-11-05/improvement-plan-2026-11-05.md](archive/2026-11-05/improvement-plan-2026-11-05.md)  
> 目的: 採用マイナス ~-23 を潰し、正味 ~+109 をさらに押し上げる。

---

## 方針

1. **見る権利・実行権・定義書き込み権を分離する**（HostDelegation + scene overlay）
2. **通常 UI ホストの資格情報寿命を閉じる**（親 client broker / IPC）
3. **XPBD を「動く」だけでなく「接触する・確保しない」まで引き上げる**
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
| P1-1 | kick API + friendship → epoch | -2 | `POST /rooms/:id/room_key/rotate`（owner・RL）。auth relation 変更で CAS ローテ | Friends 解除後、旧鍵で新 epoch 平文が読めない否定系 |
| P1-2 | host で XPBD body-body 接触 | -2 | `step_dt` に bounds 渡し。未使用 `rapier3d` を外すか理由コメント。overview/client.md を XPBD に更新 | 2 箱が重ならない回帰テスト |
| P1-3 | hot-path zero-allocation | -2 | `clone_from` / buffer swap、彩色マーカー、LBVH scratch 常駐。1k entity alloc count | tick 確保回数が上限以下のテストまたは bench 記録 |
| P1-4 | handover 候補 accept 二段階 | -2 | Presence 先頭指名をやめ、候補 accept 後に CAS。不在時は即 dormant | 非 accept 候補に lease が付かない否定系 |
| P1-5 | SceneBudget | -2 | entries / JSON bytes を protocol decode 上限以下で原子検査 | 上限超過 413/422 + 競合テスト |
| P1-6 | refresh token を OS vault へ | -1 | prefs は key id のみ。不能時は session-only | 平文 refresh がディスクに残らない |

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
| P3-1 | verifier Spot-Check orchestration | 目標 | Noop 配線のみ OSS |
| P3-2 | Frame 補間 / jitter buffer | 提案 | Epoch Prev/Curr |
| P3-3 | S2S 連合 / Spatial VCS 本体 | 目標 | Origin Cert v1 が素地 |
| P3-4 | billing / market / Relay QoS | 目標 | ディレクトリ未設置 |
| P3-5 | Formula / Wake / OSC / LOD / audio | 目標 | 減点しない |
| P3-6 | 署名付き installer / 自動更新 | 提案 | Windows release 延長 |

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
3. **XPBD 接触 + zero-alloc**（P1-2 / P1-3）を並行
4. **kick/epoch + SceneBudget + handover accept**（P1-1 / P1-4 / P1-5）
5. **UI 分割 / docs / CI フレーク**（P2）

スライス完了条件に必ず含める:

- **新しい保証は否定系 E2E で確認**
- **保証表とコードが一致**（特に権限表・物理非保証）
- **新クレートは CI・cargo deny・テスト >0**（再発防止）
