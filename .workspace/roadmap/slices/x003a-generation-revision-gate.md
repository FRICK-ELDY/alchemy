# x003a — generation / revision 単調ゲート（replay 拒否）

> **ID:** `x003a` · **slug:** `generation-revision-gate` · **status:** `done` · **area:** `[protocol, world, host, client, docs]` · **depends_on:** `[x0039]`

目標: 有効期限内の古い署名済み Manifest / Presence の replay で、現 Frame 経路を止めたり旧 host 鍵へ巻き戻したりできない。`(room, host_generation, revision)` の後退を client が拒否する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-3**

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [trust-boundary.md](../../0_doc/architecture/trust-boundary.md) · [client.md](../../0_doc/architecture/client.md)

## 設計

| 層 | 内容 |
|:---|:---|
| protocol | ManifestSeal / Presence（ワイヤ）に `host_generation`。revision は既存を単調増として契約化。`generation_gate` ヘルパ |
| world | seal / attestation 発行時に generation を claim に載せる。Room に永続；handoff（鍵変更）時のみ新 generation |
| host | 起動・attest 取得で generation を固定。Manifest / Presence に載せ単調に bump（revision） |
| client | 受信 `(generation, revision)` が既知より後退なら **discard**（x0038）。採用時のみ状態更新 |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | proto / seal claim / PIN | done |
| 2 | world + host: generation 配線 | done |
| 3 | client: 単調ゲート + 単体テスト | done |
| 4 | 否定系: protocol/client 単体（有効旧 seal 相当の gen/rev 後退）+ e2e-probe（seal 無し junk 後も host Frame 継続） | done |
| 5 | architecture 更新 | done |

## 契約（固定したい判断）

1. **同一 generation 内**では revision は単調非減少。後退は discard  
2. **同一 generation 内**では `host_pub` は固定。不一致は discard（Frame 鍵すり替わり防止）  
3. **generation 切替**は world が認めた新世代証明があるときだけ（handoff）。クライアントが勝手に世代を上げない  
4. 古いが署名有効な Manifest を再適用しても Frame 検証用 `host_pub` を巻き戻さない  
5. world `claim_host_generation` は CAS（owner 二重 host 起動オーバーラップを脅威に含む）  
6. x0038 の discard 契約の上に乗る（失敗で leave しない）  

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 古い署名 Manifest の replay で現行 Frame 経路が止まらない | 完全な host handoff UX / Wake-on-Join |
| 同一 generation 内の `host_pub` すり替わりを client が discard | 二重 host 継続時の generation フリップフロップ（負け側再 `host_keys` が gen を積む） |
| Presence の revision 後退でスロット状態が巻き戻らない | Zenoh 配送順序の保証 |
| seal claim の generation とワイヤ状態の一致 | transport ACL |

## やらない

- decode limit / prod S2S / test DB（→ x003b）
- Formula VM / verifier 本体
- mTLS

## 成功条件

1. 以前に受理した `(generation, revision)` より古い署名 Manifest を再送しても client は適用しない（**単体**: `generation_gate` + client gate）  
2. 現行の検証済み `host_pub` / Frame 受信が継続する  
3. handoff 以外で generation が勝手に上がらない（world CAS + client 同一 gen `host_pub` 固定）  
4. protocol/client/host テスト緑 + e2e-probe が junk Manifest 後も host Frame 継続を確認（**署名付き旧 seal の GUI client discard は hermetic E2E では未計測**；単体が正本）  

← [目次](../README.md)
