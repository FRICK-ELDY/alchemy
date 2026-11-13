# x009f — 親 client 資格情報 broker（P0-2）

> **ID:** `x009f` · **slug:** `parent-credential-broker` · **status:** `done` · **area:** `[client, host, protocol, docs]` · **depends_on:** `[x0032, x0025, x003d]`  
> **優先度:** improvement-plan P0-2（UI ホストの access JWT 寿命）

目標: GUI ログイン由来の短命 access JWT を子 host に一度きり渡して終わりにせず、**親 client が refresh を保持したまま** loopback IPC で更新済み access JWT を push する。host は親からの供給で Room Token を再発行でき、hard expiry 前に必ず再供給される。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P0-2 · Opus/GPT「UI host access JWT 寿命」· `host_supervisor.rs` 非保証「Room Token reissue fails after access JWT TTL」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| UI 注入パス（親に `AUTH_USER` / 既存 `ALCHEMY_AUTH_TOKEN` が無い）では host を **ParentBroker** モードで起動。`refresh_token` は子に渡さない | 親が落ちたあとの子単独 refresh；OS vault（P1-6） |
| 親は access JWT 更新のたびに（login / register / silent·remember-me 適用後 / soft-refresh / Host 起動直後·同一 room）`AUTH_PUSH`。**`auth` lock を握ったまま IPC しない**（clone してから retry）。Leave/Logout は host 維持のため、**再ログイン時 push は必須** | WAN / 非 loopback IPC；TLS；失効済み JWT の push；IPC `OK`＝inbox 受理のみ（非 ParentBroker は apply 拒否・消費済み — 親は durable 子へ push しないことで対称） |
| host は push を受理すると auth セッションと hard expiry を更新し、続く host Room Token 再発行に新しい Bearer を使う | JWT 署名検証（親プロセス信頼境界内・IPC トークン認証のみ） |
| ParentBroker 時、host は auth soft-refresh で login / env 再読込をしない（親待ち）。auth hard expiry 後は既存どおり配信停止 | `AUTH_USER`(+password) / 長命 env `ALCHEMY_AUTH_TOKEN` 経路の変更（従来どおり host 自己 refresh） |
| 否定系: 偽 IPC トークンの `AUTH_PUSH` は無視。統合: 短 TTL でも親 push 後に host Token 再発行が成功する（実時計 30 分待ちはしない） | refresh 平文 prefs → OS vault |

### IPC 行（protocol）

```
AUTH_PUSH <ipc_token> <expires_in> <access_jwt>\n
```

成功応答 `OK\n` / 拒否 `ERR\n`。`expires_in` は残り秒（≥1）。`access_jwt` は空白を含まない。

### 起動 env（UI 注入時のみ）

| 変数 | 意味 |
|:---|:---|
| `ALCHEMY_AUTH_TOKEN` | 初回 bootstrap 用 access JWT（従来） |
| `ALCHEMY_AUTH_BROKER=1` | ParentBroker（自己 refresh しない） |
| `ALCHEMY_AUTH_EXPIRES_IN` | 初回残り TTL 秒（hard expiry 起算） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | protocol `AUTH_PUSH` + host IPC inbox | **done** |
| 2 | host credential chain ParentBroker + worker 適用 | **done** |
| 3 | HostSupervisor push + AuthGate remaining TTL + token-refresh 連携 | **done** |
| 4 | 否定系 / 統合テスト + 保証表 | **done** |

## 完了条件

1. GUI 注入 host が ParentBroker で立ち上がり、親 `AUTH_PUSH` 後に host Room Token 再発行が新しい Bearer を使う（`parent_broker_push_schedules_host_token_refresh_plan`）。
2. `refresh_token` が子 env / IPC に出ない。PasswordSession への誤 push は拒否。
3. 偽 IPC トークン / inbox poison の push は `ERR`。
4. login / register / soft-refresh / boot silent 適用後に、`auth` lock 外で `push_access_token_retry`。`expires_in` 無しの UI inject / 同一 room Host は拒否。
5. architecture（client / trust-boundary）が x009f と一致。
