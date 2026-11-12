# x009d — canonical scene owner/editor 門番（P0-1 前半）

> **ID:** `x009d` · **slug:** `host-delegation-scene-overlay` · **status:** `done` · **area:** `[world, docs]` · **depends_on:** `[x0080, x0081, x0082, x0098]`  
> **優先度:** improvement-plan P0-1（定義書き込み権の分離）  
> **着地範囲:** Phase 0–2。Phase 3–4 は [x009e](x009e-session-overlay-eligible-host.md)。

目標（本 PR）: **canonical scene の append/delete を `owner_sub` ∪ `editor_subs` に限定**し、非 writer の takeover host を 403 にする。入室権（visibility）と計算ホスト権（active_host）と定義書き込み権をコード上で分離する。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P0-1 · Opus/GPT 双方の「takeover host が canonical scene を書ける」弱み · Resonite Dash Session Permissions（Admin≈owner / Builder≈editor）

製品語彙（UI 後続）: **Admin = `owner_sub`** · **Builder = `editor_subs`**。Moderator / Guest / Spectator は本 Slice の保証外（後続）。

---

## 保証する / しない（Phase 0–2 着地）

| 保証する | 保証しない |
|:---|:---|
| **権限表**（下表・本着地列）が world HTTP と一致。canonical `POST/DELETE …/scene/entries` は `owner_sub` または `editor_subs` のみ（かつ host Token + PoP + active_host） | Dash Permissions UI；Moderator キック／サイレンス；Spectator 専用 Intent |
| 非 owner/editor の takeover host が canonical scene を append/delete すると **403** `scene_writer_forbidden` | 偽 Notice 単独での権限昇格 |
| `editor_subs` は owner のみ `Rooms.set_editor_subs/3` で更新（HTTP CRUD は後続）。空なら writer は owner のみ | Group / 組織 ACL；期限付き editor capability |
| 否定系: public Room を非 owner が takeover → scene append/delete 403 | **`session_overlay`**（Phase 3 後続）；**eligible_host / HostDelegation**（Phase 4 後続）；visibility のみでの takeover 制限；inventory Dual-auth が非 writer host 経由で canonical に書くこと（overlay 導入まで非保証） |
| 非 editor への handover/takeover 後は **canonical 書き込みが凍結**（owner は active_host でない／host は writer でない）。owner が再 Host するか editor に載せるまで永続編集不可 — **Phase 0–2 で製品として許容** | 凍結中の一時編集（→ Phase 3 overlay）；`ensure_*` と CAS のあいだの editor 剥奪 TOCTOU（active_host 門番と同粒度・許容） |

### 権限表（正本）

| 操作 | owner (Admin) | editor (Builder) | active_host のみ | guest |
|:---|:---|:---|:---|:---|
| 入室（visibility ACL） | ✓ | ✓※ | ✓※ | ✓※ |
| Host Token / simulate / seal / attest | ✓（active 時） | ✓（active 時） | ✓ | ✗ |
| canonical scene append/delete（Save） | ✓ | ✓ | ✗ | ✗ |
| `editor_subs` 変更 | ✓ | ✗ | ✗ | ✗ |
| `session_overlay` / eligible takeover | —（後続） | — | — | — |

※ visibility を満たす場合。editor であることは入室権を自動付与しない。

---

## 設計（要点・本着地）

```
visibility ACL ──► 入室・guest Token
active_host ──► Host Token / simulate / seal（定義は書かない）
owner_sub ∪ editor_subs ──► canonical scene 書き込み（永続）
```

後続（別 PR）:

```
eligible_host / HostDelegation ──► takeover（Phase 4）
active_host ──► session_overlay（Phase 3・dormant 破棄）
GET scene / ManifestSeal ──► canonical ⊕ overlay（Phase 3）
```

### データ（world・本着地）

| フィールド | 意味 |
|:---|:---|
| `editor_subs` | JSON 配列（文字列 sub）。owner が `set_editor_subs/3` で管理 |

### API（本着地）

| Phase | 経路 | 認可変更 |
|:---|:---|:---|
| 1 | `POST/DELETE …/scene/entries` | `ensure_active_host` **かつ** `ensure_scene_writer` |
| 2 | `Rooms.set_editor_subs/3` | owner のみ（HTTP は後続） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + 権限表（本ファイル） | **done** |
| 1 | `ensure_scene_writer` + scene HTTP 門番 + 否定系（非 owner takeover → 403） | **done** |
| 2 | `editor_subs` 列 + `Rooms.set_editor_subs/3`（owner）+ editor が host 時は scene 可 | **done**（HTTP CRUD は後続） |
| 3 | `session_overlay` + dormant 破棄 + seal/GET 合成；inventory → overlay | **deferred**（後続 PR） |
| 4 | `eligible_host_subs` / HostDelegation + takeover CAS | **deferred**（後続 PR） |

---

## 完了条件（本 PR = Phase 0–2）

1. 非 owner/editor が public Room を takeover して Host Token を得ても、`POST/DELETE …/scene/entries` は **403** `scene_writer_forbidden`。
2. owner（および editor）が active host のとき、従来どおり CAS append/delete 成功。
3. architecture（world-server / trust-boundary）の保証表が本着地の権限表と一致。

後続 PR の完了条件（参考）: dormant 後に overlay が canonical に残らない／visibility  alone で非 eligible が takeover できない。

## やらない（本 PR）

- `session_overlay` / HostDelegation / eligible_host（Phase 3–4）
- Dash Room / Permissions タブ UI（P2-1 寄）
- Moderator ロール・kick API 本体（P1-1）
- SceneBudget（P1-5）
- Proprietary Spot-Check
