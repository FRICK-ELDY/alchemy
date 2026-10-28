# Alchemy — 提案（0点）統合一覧（まとめ / 2026-10-26）

> 未実装・評価対象外だが実装すれば価値が上がる項目。加点も減点もしない。
> 両者合意を優先し、片方のみは注記。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 両者合意の提案

### 信頼境界・門番

- **永続 Room capability + 5段階公開範囲** `0`
  > owner / invite / role を永続化し、Token は capability 判定結果のみ署名。Friends は auth Friendship 参照。
  > 対象ファイル: `world-server/lib/world/`

- **CasObject と BlobGrant（所有者/license）の二層化** `0`
  > global dedupe と GET 認可・Quota 単位を両立。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **署名付き SceneManifest（world 定義 → host 実行）** `0`
  > 直書きコンテンツ解消と client 側改ざん検知の両方に効く。
  > 対象ファイル: `protocol/proto/scene_manifest.proto`, `world-server/`, `client/host-server/`

- **verifier 最小 Spot-Check** `0`
  > 独立 app。移動量上限などルール1本から。
  > 対象ファイル: （新規）`world-server` verifier

- **WAN mTLS/QUIC + Relay Agent / namespace ACL** `0`
  > host の Room Token だけでは Frame/control を守れない。
  > 対象ファイル: `zenoh-router/`

### 同期・体感

- **snapshot queryable + 純粋デルタ** `0`
  > late join / packet loss 収束。箱毎 tick upsert の回避策を外す。
  > 対象ファイル: `protocol/`, `client/host-server/`

- **jitter buffer・補間・限定予測（60–120fps）** `0`
  > 対象ファイル: `client/app/`

- **simulation と network publish の bounded pipeline** `0`
  > 60Hz を Zenoh `.wait()` から切り離す。
  > 対象ファイル: `client/host-server/src/main.rs`

### コア拡張（目標どおり・減点しない）

- **Friendship / Group / ToS version migration** `0` — `auth-server/`
- **presigned direct PUT + 軽量検疫** `0` — `assets-server/`
- **Spatial VCS / Origin Cert（別保証単位）** `0` — `assets-server/`
- **Formula VM / Wake-on-Join / host handoff** `0` — `client/host-server/`
- **OSC Device Input Edge / Dual Editor / LOD cache** `0` — `client/app/`
- **audio / tracking 独立 schema** `0` — `protocol/proto/`
- **launcher 最小プロセスマネージャ** `0` — `launcher/`
- **Hermetic Compose E2E + input fuzz/proptest** `0` — `.github/`, `client/host-server/`
- **buf lint / breaking / golden corpus** `0` — `protocol/`
- **OpenTelemetry / SLO** `0`
- **audit/deny・SBOM・署名配布** `0`
- **shared-sim crate / typed EntityKind** `0`

---

## 片方のみ・採用する提案

| 提案 | 出典 | 採用理由 |
|:---|:---|:---|
| auth 失効の下流伝播（revocation feed） | Opus | logout 後 900s 有効の穴を塞ぐ |
| カメラ相対描画で f64 契約を活かす | Opus | binary64 保証の実質化 |
| quantum 圧縮（回転追加時） | Opus | 帯域 |
| cloudflared HA / WAF as code | GPT | 運用再現性 |
| trust-boundary threat model を architecture 正本へ | GPT | 今回の最大論点を文書化 |
| DPoP 風セッション鍵（input なりすまし対策） | Opus 弱み改善に近い | 短期は ACL、中期は cnf |
| alchemy_auth_client 共有パッケージ | Opus | JWKS 複製解消 |

---

## 提案の優先度（まとめ推奨）

1. 署名 Manifest + Room capability（定義 vs 実行・門番）
2. CasObject/BlobGrant + GET 認可の明文化または実装
3. snapshot + client 署名/sequence 検証
4. WAN 相互認証・ACL
5. launcher 最小実装 + Compose E2E
6. Friendship / verifier / Formula（ロードマップ順）
