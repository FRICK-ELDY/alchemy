# x007d — Live Manifest mesh remount

> **ID:** `x007d` · **slug:** `live-manifest-mesh-remount` · **status:** `active` · **area:** `[client, docs]` · **depends_on:** `[x003c, x0048, x006c, x007c]`  
> **優先度:** UX 本線（MainHub 空シェル + Join/Host 後の x003c assets GET）

目標: 未入室は **system_ui + クリア色のみ**（Coin / 床なし）。Join/Host 後に受理した SceneManifest の mesh blake3 → assets GET（PlaceRead 可）→ **GPU remount** で空間 mesh を表示。Leave で空シェルへ戻す。

根拠: silent auto-join 廃止後、boot 常時 Coin と「remount non-goal」が衝突。x003c の authenticated GET 本線を UI Join 経路に復活させる。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| MainHub / Step1–2: 世界 mesh・床を描画しない（空シェル） | Quest 単独の別ホーム空間 |
| 受理 Manifest の primary mesh blake3 → assets GET → `SceneGpu::replace_world_mesh` | Manifest 全 mesh entity のマルチ GLB |
| GET 失敗は placeholder + sticky status（x0048）；プロセスは落とさない | mid-download cancel UI / HTTP abort；同一 blake3 のセッション内再試行（Leave / re-Join で回復） |
| 新しい blake3 なら再 remount；同一 blake3 の revision のみはスキップ；`try_send` 成功後のみ hex を記録 | Frame Live と mesh 到着の厳密同期（Waiting は frame 用のまま） |
| Leave / Logout 後 GPU を空シェルへ戻す；in-flight remount は `try_apply_world_remount`（世代 + `wire_cap` + remounted/status と queue 原子）で破棄。Leave の bump/clear も pending 同ロック | leave_room_local 内での wgpu 直接操作；Join 単位のワーカー cancel（世代ゲートで十分） |
| host-generation 切替は bump + `remounted` クリアを pending 同ロックで行い、旧 GPU mesh は新 GET 完了まで残す | 切替瞬間の空シェルフラッシュ；discard 後の mesh / `remounted` 誤適用 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | empty shell boot + draw スキップ | **done** |
| 2 | `replace_world_mesh` + SharedState pending epoch | **done** |
| 3 | `Some(blake3_tx)` → load_auth_mesh → remount | **done** |
| 4 | Leave → empty；docs / 保証一致 | **done** |

---

## 完了条件

1. silent login → MainHub にコインも床も出ない。
2. Host / Join 後、ログに `downloaded blob via assets GET` と Manifest blake3；空間 mesh が表示される。
3. Leave 後、再び空シェル（コイン無し）。
4. architecture / テスター手順が上記と一致。

## 関連

- [x003c-assets-required-mesh.md](x003c-assets-required-mesh.md)
- [x007c-worlds-catalog-join-host.md](x007c-worlds-catalog-join-host.md)
- [client.md](../../0_doc/architecture/client.md)

← [目次](../README.md)
