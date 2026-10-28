# x003a — generation / revision 単調ゲート（replay 拒否）

> **ID:** `x003a` · **slug:** `generation-revision-gate` · **status:** `planned` · **area:** `[protocol, world, host, client, docs]` · **depends_on:** `[x0039]`

目標: 有効期限内の古い署名済み Manifest / Presence の replay で、現 Frame 経路を止めたり旧 host 鍵へ巻き戻したりできない。`(room, host_generation, revision)` の後退を client が拒否する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-3**

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [trust-boundary.md](../../0_doc/architecture/trust-boundary.md) · [client.md](../../0_doc/architecture/client.md)

## 設計

| 層 | 内容 |
|:---|:---|
| protocol | ManifestSeal / Presence（必要ならワイヤ）に `host_generation`。revision は既存を単調増として契約化 |
| world | seal / attestation 発行時に generation を claim に載せる。handoff 時のみ新 generation |
| host | 起動・attest 取得で generation を固定。Manifest / Presence revision を単調に bump |
| client | 受信 `(generation, revision)` が既知より後退なら **discard**（x0038）。採用時のみ状態更新 |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | planned |
| 1 | proto / seal claim / PIN（必要時） | planned |
| 2 | world + host: generation 配線 | planned |
| 3 | client: 単調ゲート + 単体テスト | planned |
| 4 | 否定系（旧署名 Manifest replay）E2E | planned |
| 5 | architecture 更新 | planned |

## 契約（固定したい判断）

1. **同一 generation 内**では revision は単調非減少。後退は discard  
2. **generation 切替**は world が認めた新世代証明があるときだけ（handoff）。クライアントが勝手に世代を上げない  
3. 古いが署名有効な Manifest を再適用しても Frame 検証用 `host_pub` を巻き戻さない  
4. x0038 の discard 契約の上に乗る（失敗で leave しない）  

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 古い署名 Manifest の replay で現行 Frame 経路が止まらない | 完全な host handoff UX / Wake-on-Join |
| Presence の revision 後退でスロット状態が巻き戻らない | Zenoh 配送順序の保証 |
| seal claim の generation とワイヤ状態の一致 | transport ACL |

## やらない

- decode limit / prod S2S / test DB（→ x003b）
- Formula VM / verifier 本体
- mTLS

## 成功条件

1. 以前に受理した `(generation, revision)` より古い署名 Manifest を再送しても client は適用しない  
2. 現行の検証済み `host_pub` / Frame 受信が継続する  
3. handoff 以外で generation が勝手に上がらない  
4. 否定系 E2E（replay）緑 + protocol/client/host テスト緑  

← [目次](../README.md)
