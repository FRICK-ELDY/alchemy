# Alchemy — 提案（0点）統合一覧（まとめ / 2026-11-12）

> 根拠: [Opus](opus/opus-specific-proposals-2026-11-12.md) · [GPT](gpt/gpt-specific-proposals-2026-11-12.md)  
> 減点せず「次のステップ」として記録。新規依存は人に回す。前回提案のうちコードで閉じたものは載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 実装すれば価値が上がる提案 |

---

## P0 に直結する設計提案

- **HostDelegation / scene 永続書き込みと session overlay の分離** `0`（双方）
  > takeover は owner 署名 capability または eligible_host + candidate opt-in 必須。canonical scene は owner/editor のみ。active host は `session_overlay`（dormant で破棄）だけを持つ。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **親 client を短命資格情報 broker にする（IPC push）** `0`（双方）
  > refresh / password は子に渡さず、親が access JWT を loopback IPC で更新配布。
  > 対象ファイル: `client/app/src/host_supervisor.rs`, `client/host-server/src/host_ipc.rs`

---

## 移譲・鍵・資源

- **kick / friendship 解除 → Room key epoch イベント連携** `0`（双方）
- **SceneBudget（entries / JSON bytes）を protocol decode 上限と共有** `0`（GPT）
- **handover を候補 accept 二段階 CAS にする** `0`（GPT）
- **Room epoch の push ヒント / 無停止二相切替** `0`（双方）
- **OS credential vault 抽象** `0`（GPT）

---

## 物理・シミュレーション

- **host で XPBD body-body 接触を有効化 + zero-allocation tick / 規模別 benchmark** `0`（双方）
  > 対症療法の bounds 渡しでは閉じない。[`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) Phase 1（既存 `shared-sim` 進化）の DoD に吸収。着手は improvement-plan P1-1 / P1-4 / P1-5 の後。
- **Frame 補間・jitter buffer を Epoch Prev/Curr へ接続** `0`（双方）
  > [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md) Phase 2（`wgpu` Compute）に吸収。Phase 1 Green 後。

---

## 門番・保守

- **scene append 時の Grant 照会 / HTTP PoP nonce** `0`（Opus）
- **Desktop / Dash のイベント→コマンド分離、Rooms モジュール分割** `0`（Opus）
- **assets CI 用 `_build` ボリューム分離** `0`（Opus）

---

## 目標・長期（減点しない）

- **verifier Spot-Check オーケストレーション（OSS は配線のみ）** `0`
- **S2S 連合 / Spatial VCS 本体 / Formula / OSC / Wake-on-Join / audio** `0`
- **relay 帯域 QoS・SLO dashboard** `0`
- **署名付き installer / 自動更新 / host バイナリ hash 検証** `0`
- **Optional Profile（billing / market）は実装時まで対象外維持** `0`
