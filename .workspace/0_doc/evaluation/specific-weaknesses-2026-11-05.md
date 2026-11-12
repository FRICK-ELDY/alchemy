# Alchemy — マイナス点統合一覧（まとめ / 2026-11-05）

> 根拠: [Opus](opus/opus-specific-weaknesses-2026-11-05.md) · [GPT](gpt/gpt-specific-weaknesses-2026-11-05.md)  
> 採用ルール: 双方合意は折衷または厳しめ側、片方発見の実害は採用。前回まとめ（[archive/2026-10-30](archive/2026-10-30/specific-weaknesses-2026-10-30.md)）で **closed** と確認された項目は載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

| 採用合計（概算） | **-24** |

---

## P0 — 新信頼境界（Room 鍵寿命・host Token・scene 原子性）

### world / host / relay

- **host Room Token が bearer のまま Zenoh 制御面に載り、world host API は PoP を見ない** `-3`（Opus -3 / GPT 未個別）
  > inventory spawn/despawn が host Room Token を Room 共有鍵 AEAD で包んで put する。部屋メンバーは復号でき、host 証明書は自室作成で得られる。world の `host_keys` / seal / scene は Token + owner 検査のみで `cnf.spk` PoP 無し。盗んだ host Token で attestation・generation・canonical seal を正規に取り、host を乗っ取れる連鎖。release GUI は guest Token のみなので潜在欠陥だが、プロトコルと API がこの形で確定している。
  > 採用: **-3**（Opus）
  > 対象ファイル: `client/app/src/inventory.rs`, `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/token.ex`, `relay-server/config.wan.json5`

- **Room 鍵が部屋寿命で固定され、visibility 変更・kick 後も過去参加者が復号できる** `-3`（GPT -3 / Opus -2）
  > `room_key` は作成時に一度だけ生成され、Token 発行ごとに同じ鍵を返す。WAN guest は `*/*/manifest|frame|presence` をワイルドカード購読できるため、一度入った鍵保持者は後から private / friends にしても平文を読める。AEAD は現在の他室隔離を閉じたが、revoke 境界ではない。
  > 採用: **-3**（GPT 厳しめ）
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`, `relay-server/config.wan.json5`

- **Scene append / delete が revision CAS なしの RMW で更新を失う** `-3`（GPT -3 / Opus 未個別）
  > `append_scene_entry` / `delete_scene_entry` は読んだ revision を WHERE に含めない。host の spawn/despawn は別スレッド HTTP なので、同時要求で後勝ち・片方が恒久欠落しうる。canonical seal の定義元で欠落が起きると、下流署名が正しくても失われた状態を正当化する。
  > 採用: **-3**（GPT）
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `client/host-server/src/inventory_spawn.rs`, `client/host-server/src/inventory_despawn.rs`

- **relay-agent 互換経路が今も `key_pem` を cleartext HTTP で返す** `-2`（GPT -2 open / Opus -1 partial）
  > 正経路は world CSR（鍵はネットワークに出ない）。Agent はサーバー生成鍵を JSON で返し、非 loopback も warn のみ。互換を残す限り前回 P0-4 は閉じ切っていない。
  > 採用: **-2**（GPT。正経路 closed を認めつつ互換を減点）
  > 対象ファイル: `relay-agent/src/lib.rs`

---

## P1 — 門番衛生・資源上限・テスト穴

- **host Room Token 系 HTTP に rate limit が無い** `-2`（GPT -2 / Opus -1）
  > x0071 の RL は auth-JWT scope のみ。`host_keys` / `manifest_seal` / scene / PlaceRead は無制限。seal-hash Port 直列と組み合わさると自室連打で全部屋の配信再開を止められる。
  > 採用: **-2**
  > 対象ファイル: `world-server/lib/world_web/router.ex`

- **外部 GLB の展開後資源上限が無い** `-2`（GPT -2 / Opus 未個別）
  > upload 100 MB 制限のあと、client は頂点・index・画像を上限なしで確保。正当登録の圧縮爆弾で OOM しうる。
  > 採用: **-2**
  > 対象ファイル: `client/app/src/mesh.rs`

- **assets `mix test` が既定並列でフレークする** `-1`（Opus -1 / GPT は減点せず記録）
  > CI all 初回 54 failures（`Database busy` / `cas_test` rm_rf）。単独再実行は 0。test でも reaper/GC が動き共有パスと競合。
  > 採用: **-1**（再現フレークとして採用。恒常バグとはしない）
  > 対象ファイル: `assets-server/lib/assets/application.ex`, `assets-server/config/test.exs`

- **Allocation に主体別 throughput policy が無い / WAN E2E が実 host publish まで未到達** `-1`（双方関連）
  > Agent の同時上限は閉じた。時間窓 bucket は未。trust-probe WAN は synthetic Manifest put で、credential worker〜実 Frame 一往復は非保証。
  > 採用: まとめて **-1**
  > 対象ファイル: `relay-agent/src/lib.rs`, `docker-compose.e2e.wan.yml`, `client/trust/src/bin/trust-probe.rs`

---

## P2 — 品質・DX・残債

- **軽微群** `-1` 帯（採用合計 **-6**）
  > BLAKE3 / seal Port 直列、mime 先着、Origin Cert が内容ハッシュを署名しない、`/health` が Port 非反映、共有 JWKS 失敗クールダウン無し、~~`USER_ROOT_SLOTS_MAX=4`~~（x009b closed）、host `main()` 肥大、配信 tick `Vec`、bind 自己申告キー、take-out UI と guest Token の不整合、dash/desktop 一枚岩、腐った smoke、`ci.sh` が shared 未収録、client.md / wan コメント / world 保証表鮮度、`vision.md` 空、Room 上限の並行 overshoot、Windows XR loader flake。
  > 採用: まとめて **-6**

---

## スコア感（採用マイナス）

| 帯 | 点数 |
|:---|---:|
| P0 新信頼境界 | ≈ -11 |
| P1 衛生・資源・テスト | ≈ -7 |
| P2 品質・DX | ≈ -6 |
| **合計概算** | **≈ -24** |

前回まとめ採用マイナス ≈ -28 から微減。Relay / CAS の旧 P0 は閉じ、穴は **Room 鍵寿命・host Token の運び方・scene CAS** に移った。
