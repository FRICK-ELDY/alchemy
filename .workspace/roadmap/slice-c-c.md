# Slice C-C — Manifest（低頻度）＋ RenderFrame デルタ（高頻度）（完了）

目標: **「何を置くか」と「どこにいるか」をチャネル分離**する。  
低頻度の Scene Manifest で `entity_id → blake3` を共有し、高頻度の RenderFrame は **動いた Entity の pose 差分だけ**送る。GLB 本体は引き続き assets-server（host 非経由）。

正本: [protocol.md](../0_doc/architecture/protocol.md) · [client.md](../0_doc/architecture/client.md) · overview ⑧

## 設計方針

| チャネル | 頻度 | 中身 | 載せないもの |
|:---|:---|:---|:---|
| Scene Manifest | 入室・スポーン・台帳更新時（host は 5s 再送で late joiner 補助） | `entity_id` → `blake3`（+ 任意の初期 pose） | GLB バイト・毎フレーム再送 |
| RenderFrame | 20/30/60Hz | 動いた id の pose（upsert / remove） | blake3・アセット本体・静止 Entity のフル再送 |
| assets-server | 必要時 | GLB 本体（C-B 経路を再利用） | — |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `protocol/`: `SceneManifest` 新設 + `RenderFrame` デルタ形（upserts/removes）+ `PROTOCOL_PIN` → `0.3.0-slice-c-c` | done |
| 1 | Manifest 縦: host が `…/manifest` Publish → client 購読 → blake3 で assets download → BLAKE3→gltf→wgpu | done |
| 2 | デルタ縦: host が静止を省略し動いた Entity のみ Publish → client はローカル表を upsert/remove | done |
| 3 | smoke-manifest-delta + docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Manifest と RenderFrame の役割分離（低頻度参照 / 高頻度 pose） | Manifest の world-server 署名・HTTPS 正本化（後続） |
| 同一 room の client が Manifest の blake3 で同じ Blob を取得できる | LOD / KTX2 / P2P キャッシュ / Vault・Commit |
| 静止 Entity を毎 tick フルスナップショット再送しない | Rapier / Formula / 本格ビット圧縮 |
| host に GLB バイトを載せない | verifier / 公式リレー / launcher |

## やらない（後続）

- world-server 署名付き Scene Manifest（概要図の空間台帳本実装）
- R2 / NAS・Origin Cert
- 予測補間の本格化・可変レート UI
- client submodule 化

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
bash .scripts/smoke-assets-glb.sh
zenohd -c zenoh-router/config.json5
bash .scripts/smoke-manifest-delta.sh
cd client
cargo run -p host-server   # Manifest + デルタ RenderFrame
cargo run -p client        # Manifest → download → デルタ適用
```

キー式:

- manifest: `alchemy/dev/room/local/manifest`
- frame: `alchemy/dev/room/local/frame`
- input: `alchemy/dev/room/local/input`

← [目次](README.md)
