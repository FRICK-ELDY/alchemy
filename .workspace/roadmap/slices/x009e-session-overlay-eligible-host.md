# x009e — session_overlay + eligible_host（P0-1 後半）

> **ID:** `x009e` · **slug:** `session-overlay-eligible-host` · **status:** `active` · **area:** `[world, host, docs]` · **depends_on:** `[x009d, x0080, x0082]`  
> **優先度:** improvement-plan P0-1 後半（[x009d](x009d-host-delegation-scene-overlay.md) Phase 3–4）

目標: active host の一時 entity を **`session_overlay`** に閉じ（dormant で破棄）、GET scene / ManifestSeal は canonical ⊕ overlay を合成する。inventory spawn/despawn は overlay HTTP へ。takeover は **owner ∪ `eligible_host_subs`** に限定する。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST/DELETE …/overlay/entries` は active_host のみ（scene writer 不要）。**atomic packed CAS**（SQL が canon+overlay 双方を WHERE） | overlay → canonical の Save/merge UI |
| GET `…/scene` と ManifestSeal は **composed**（canonical entries ∥ overlay entries）。`revision` は pack(canonical, overlay) | PlaceRead Grant が overlay-only hash を永続保証すること |
| **ホスト交代 / dormant** で overlay entries を破棄し **overlay revision を +1**（packed 単調・GenRev 後退なし）。対象: dormant・lease expire・takeover・expired activate・**nominate handover**。self-renew は保持 | crash 瞬間の in-flight overlay HTTP；同一 revision での content 差替え |
| inventory host HTTP は **overlay** 経路（session 寿命）。非 writer takeover host でも take-out 可 | セッションまたぎの inventory 永続（Save 後続） |
| takeover は visibility に加え **owner または `eligible_host_subs`**。空リスト = owner のみ。`PUT …/eligible_hosts` で owner が設定 | 署名付き HostDelegation JWT（台帳リストで代替）；candidate opt-in 二段階（P1-4） |
| 否定系: 非 eligible の takeover → 403；lease 期限切れ takeover/activate 後に前ホスト overlay mesh が残らない | Dash Permissions UI |

### packed revision

`revision = canonical_revision * 1_000_000 + overlay_revision`（各成分 < 1e6）。host は GET / mutation 応答の `revision` をそのまま CAS に使う。clear 時は revision を 0 に戻さず **bump** する（同一 `host_generation` での GenRev RejectRegression を避ける）。stride 直前の clear は **fail-closed**（`:invalid_revision`；CAS 競合の `:conflict` と区別）。compose は壊れた / overflow revision も `:invalid_revision`。lease sweeper は stride ブロックを `err_count` に計上。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `session_overlay` + overlay CRUD + compose GET/seal + dormant clear | **done** |
| 2 | host inventory → overlay URLs | **done** |
| 3 | `eligible_host_subs` + takeover 門番 + setter | **done** |
| 4 | 否定系 + 保証表 | **done** |

## 完了条件

1. 非 writer takeover host が overlay append でき、canonical append は 403 のまま。
2. dormant 後、以前の overlay blake3 が composed scene に出ない。
3. 非 owner・非 eligible の takeover → 403 `takeover_forbidden`。
4. architecture 保証表が x009e と一致。
