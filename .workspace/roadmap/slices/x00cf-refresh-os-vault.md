# x00cf — refresh token を OS vault へ（P1-6）

> **ID:** `x00cf` · **slug:** `refresh-os-vault` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x009f]`  
> **優先度:** improvement-plan P1-6（親 client の refresh 平文 prefs 排除）

目標: Remember-me の refresh token 本文を **OS vault（keyring）** に置き、`onboarding.json` には **key id のみ**残す。vault 不能時は **session-only**（ディスクに平文を残さない）。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P1-6 · [x009f](x009f-parent-credential-broker.md)「保証しない: OS vault」· GPT「OS credential vault 共通抽象」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Remember-me 永続時、refresh 本文は OS vault；prefs JSON に平文 refresh を書かない | OS vault の強度・同期・他プロセス隔離（OS 境界） |
| 起動時に旧平文 `refresh_token` があれば vault へ移し prefs から除去（migrate-once） | vault 不能環境での remember-me 永続 |
| vault 不能／エラー時は session-only（ログイン可・再起動復元なし） | headless CI / Secret Service 未起動での永続 |
| logout / `clear_session` で vault エントリ削除 + prefs key id 消去 | 子 host への refresh 配布（x009f 据え置き） |
| soft-refresh で refresh が回転したら vault を更新；**失敗時は remember-me を破棄**（旧 revoke 済みを残さない）。prefs の remember-me 更新は **IO ロック付き RMW**（remember-me OFF と競合しても key を復活させない） | プロセス内メモリ上の平文秘匿；access JWT の vault 化；vault 失敗後も次回 remember-me を維持すること；一過性 vault エラーでも remember-me を維持すること |

### keyring 境界

- service: `alchemy.refresh`
- prefs: `refresh_vault_key`（Entry の username / key id）
- 旧フィールド `refresh_token` は deserialize のみ（serialize しない）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `keyring` 依存 + `refresh_vault` ラッパ | **done** |
| 2 | prefs key id / migrate + boot / persist / logout / soft-refresh 配線 | **done** |
| 3 | 否定系単体 + architecture 鮮度 | **done** |

## 完了条件

1. Remember-me 後の `onboarding.json` に `refresh_token` 平文が無い（key id のみ）。
2. 旧平文 prefs は load 時に migrate され、再 save 後ディスクに平文無し。
3. vault 失敗時は session-only（secret / 失敗した永続を prefs に残さない）。
4. clear / logout 相当で vault エントリ削除。
5. 保証表（client / trust-boundary）と x009f 注記が一致。

## やらない

- ParentBroker / host 側の再設計
- access JWT の vault 化
- Linux CI での実 Secret Service 必須化
- keyring 以外の新規依存追加

← [目次](../README.md) · [x009f](x009f-parent-credential-broker.md) · [improvement-plan](../../0_doc/evaluation/improvement-plan-2026-11-12.md)
