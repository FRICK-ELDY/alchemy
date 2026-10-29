# assets-server（設計たたき台）

> **ステータス**: **draft** — 目標アーキテクチャのたたき台。実装正本ではない。  
> **現行正本**: [assets-server.md](assets-server.md)（GET capability モデル含む。x0003 / x001c / x001d）  
> **動機**: 現行 GET（認証済み読み取り capability）の先にある **BlobGrant / 5 層 + 外部境界** の目標像。課金・市場連動は改善計画の billing / market 帯（評価版で ID が変わりうる）  
> **関連**: [trust-boundary.md](trust-boundary.md)、[assets-threat-model.md](assets-threat-model.md)（手口×対策）、[overview.md](overview.md)

---

## この文書が保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| assets-server を **5 層 + 外部 4 境界** に分ける目標像を図示する | 現行コードがこの層構造であること |
| **BlobGrant** を「保持・利用権の SSoT」、**AssetBlob** を「物理 CAS（所有者概念なし）」と置く | GET 認可・Quota・決済の実装詳細がコードと一致すること |
| 単一 DB トランザクション境界で Drive / Comms / Vcs / Shop → Grant を結ぶ方針 | Stage 2+ PostgreSQL・Ash Domain 分割のスケジュール |
| auth / market+billing / world+zenoh / client(+盗難特徴量) を **別プロセス境界** に切る | それらのサーバーが既に存在する／本リポジトリに揃っていること |

**現行との関係**: GET capability 文書化（P1-1）と **CasObject / BlobRef + reserved put**（x003f / P1-2）は正本済み。本 draft は残る **BlobGrant / StorageVault / 5 層** の目標像。Grant 採用時は保証表とコードを揃えてから正本へ昇格する。

---

## 現状 → 目標の写像（要約）

| 現行（assets-server.md） | 本 draft |
|:---|:---|
| `CasObject`（物理）+ `BlobRef` UNIQUE(owner_sub, blake3)（x003f） | `AssetBlob`（=CasObject）+ `BlobGrant`（権利・Quota 単位）に昇格 |
| `QuotaLedger`（used + reserved） | `StorageVault`（Personal / Group）+ Grant の `quota_exempt` |
| GET: Bearer 存在のみ（所有者照合なし） | Grant 有無で GET 可否（購入・メール一時・出品ロックを含む） |
| Vault / Folder / Commit は「保証しない」 | Drive / Vcs / Shop を同一 TX 境界の上位層として置く |
| 課金・Ash・Origin Cert は後続 | market/billing・Origin Cert・client LOD を外部境界へ明示 |

---

## 目標構成図

```mermaid
flowchart TB
    %% ==========================================
    %% カラースタイル定義
    %% ==========================================
    classDef cas fill:#2b2d42,stroke:#adb5bd,stroke-width:2px,color:#fff
    classDef grant fill:#1b4332,stroke:#74c69d,stroke-width:2px,color:#fff
    classDef drive fill:#005f73,stroke:#94d2bd,stroke-width:2px,color:#fff
    classDef vcs fill:#2d6a4f,stroke:#b7e4c7,stroke-width:2px,color:#fff
    classDef shop fill:#9b2226,stroke:#f4a261,stroke-width:2px,color:#fff
    classDef auth fill:#4e2a8e,stroke:#d0bfff,stroke-width:2px,color:#fff
    classDef market fill:#7f1d1d,stroke:#fca5a5,stroke-width:2px,stroke-dasharray: 5 5,color:#fff
    classDef world fill:#3c096c,stroke:#e0aaff,stroke-width:2px,color:#fff
    classDef client fill:#b7410e,stroke:#ffd8a8,stroke-width:2px,color:#fff

    %% ====================================================================================
    %% 🌿 assets-server（単一DBトランザクション境界：Stage 1 SQLite ➔ Stage 2+ PostgreSQL）
    %% ====================================================================================
    subgraph AssetsServer["🌿 assets-server（Elixir / Ash Framework — 単一DBトランザクションで統合する5層構造）"]
        direction TB

        %% --- 第5層：個人ショップ陳列層 ---
        subgraph Layer5["🛍️ 第5層：Booth型 個人ショップ陳列層 (Domain: AlchemyAssets.Shop)"]
            Shopfront["Shopfront & ShopRelease<br/>・クリエイター個人のショップ棚 ＆ 商品ページメタデータ<br/>・不変の Git Tag (v1.0.0等) をピン留めして公開<br/>・distribution_mode: :free_direct (無料直配布) / :market_linked (有料連携)"]:::shop
        end

        %% --- 第3層 ＆ 第4層：論理コンテンツ層 ---
        subgraph Layer3_4["📁 第3・4層：論理コンテンツ・バージョン・通信保存層"]
            direction LR

            subgraph Layer3["📩 第3層：Google Drive ＆ メール兼SNS層 (Domain: AlchemyAssets.Drive)"]
                DriveNode["DriveNode (ツリー階層管理)<br/>・フォルダ / 単体ファイル(写真・音声) / Shortcut<br/>・VcsRepo(ワールド・アバター)も同一階層に配置"]:::drive
                CommsPost["CommsPost (メール ＆ 平和なマイクロフィード統合)<br/>・visibility フラグ1つで4形態に変化：<br/>  🔒 :direct (個別メール / 2段階Claim添付)<br/>  👥 :group (グループ連絡網・配布)<br/>  🤝 :friends (VR写真・Joinワープ募集)<br/>  🌐 :public (ショップ新作・アップデート告知)<br/>※トレンド・引用晒し・開放リプライは持たない"]:::drive
            end

            subgraph Layer4["🌿 第4層：Spatial Git バージョン管理層 (Domain: AlchemyAssets.Vcs)"]
                VcsCore["VcsRepo / VcsCommit / VcsTree / VcsRef<br/>・ワールド(WorldManifest) / アバター / アイテムのGit管理<br/>・Commit / Branch / Tag / 3D Diff / Merge / Rollback<br/>・📜 Origin Certificate (原産地証明：作者sub + Commit署名)"]:::vcs
            end
        end

        %% --- 第2層：容量・権利の心臓部 ---
        subgraph Layer2["⚖️ 第2層：容量(Quota)・権利台帳の心臓部 (Domain: AlchemyAssets.Vault)"]
            StorageVault["StorageVault (容量の器)<br/>・Personal ($0:1GB / $1:5GB / $5:25GB / $10:50GB / $20:100GB)<br/>・Group ($20プラン特典: 50GB共有)<br/>・支払い停止時は即時削除せず :read_only へ移行"]:::grant
            BlobGrant["🔑 BlobGrant (ブロブ保持・利用権の絶対的SSoT)<br/>・課金＆重複排除単位：UNIQUE (vault_id, blob_hash)<br/>・grant_type: :owned (自作/保存) / :purchased (購入) / :mail_temp (未保存添付)<br/>・quota_exempt: true (購入品や未Claim添付は自分の容量消費 0)<br/>・is_locked_by_sale: true (一度でも売れたTagのBlobは出品者削除を防止)"]:::grant
            StorageVault --> BlobGrant
        end

        %% --- 第1層：物理CASストレージ層 ---
        subgraph Layer1["🧱 第1層：物理CAS不変ストレージ層 (Cloudflare R2 / 自宅 QNAP NAS)"]
            AssetBlob["AssetBlob (純粋な不変バイナリ管理)<br/>・blake3_hash (PK) / byte_size / mime_type / 軽量ヘッダ検疫結果<br/>・variants (PC高画質GLB / Quest・Web軽量KTX2)<br/>※所有者・ファイル名・価格の概念を一切持たない（参照BlobGrantが0件でGC削除）"]:::cas
        end

        %% assets-server 内部の結線（すべて同一DBトランザクション内で整合性を保証）
        Shopfront -->|"特定のGit Tagをピン留め参照"| VcsCore
        Shopfront -->|"単体デジタル商品を公開"| DriveNode
        DriveNode -->|"フォルダ内でGitリポジトリを参照"| VcsCore
        DriveNode & CommsPost & VcsCore ===>|"同一DBトランザクション内で<br/>アトミックにGrant登録 ＆ 容量計算"| BlobGrant
        BlobGrant ===>|"物理ハッシュを参照 (ON DELETE RESTRICT)"| AssetBlob
    end

    %% ====================================================================================
    %% 🔌 別サーバー・別プロセスに分けるべき4つの外部領域（責務・負荷・決済の分離境界）
    %% ====================================================================================
    subgraph ExternalServers["🔌 別サーバー・別プロセスに切り出すべき4つの境界"]
        direction TB

        %% 外部①：認証・ソーシャルグラフ
        subgraph AuthServer["① 🔑 auth-server (Elixir / Phoenix / Ash — Core Profile)"]
            AuthIdentity["アイデンティティ ＆ ソーシャルグラフ SSoT<br/>・JWT発行 (RS256 / sub = user_id) ＆ /.well-known/jwks.json<br/>・📜 利用規約同意 (tos_version)<br/>・👥 Friendship(フレンド) / Follow(フォロー) / Group / Block リスト"]:::auth
        end

        %% 外部②：お金・決済・連合マーケット
        subgraph MarketAndBilling["② 🛒 market-server ＆ 💳 BillingProvider (Elixir — Optional Profile ※個人運営はOFF可能)"]
            BillingCore["決済 ＆ プリペイドコイン台帳<br/>・MoR / Stripe 連携 ＋ 180日期限コイン残高管理<br/>・サブスク課金ステータス管理"]:::market
            MarketEngine["商用マーケット決済・ライセンス・連合検索<br/>・有料アセットの価格設定・決済実行・売上14日間エスクロー<br/>・📜 License Token (購入証明) の電子署名発行<br/>・🌐 連合マーケット横断検索 (Trusted / Blocked)"]:::market
            BillingCore <--> MarketEngine
        end

        %% 外部③：リアルタイム通信・空間案内・着用権監査
        subgraph WorldAndZenoh["③ 🏰 world-server (Elixir) ＆ ⚡ zenoh-router (Rust zenohd)"]
            WorldRealtime["リアルタイム通知 ＆ 空間ゲートウェイ<br/>・🔔 メール / マイクロフィード新着時のリアルタイムPush通知 (Fan-out)<br/>・💬 容量を消費しない揮発性ルームチャット ＆ 🎙️ Zenoh音声通話<br/>・🚪 ポスト内の alchemy:// ディープリンクからの部屋ワープ解決<br/>・🛡️ apps/verifier: 着用・配置時の Origin Cert / License Token 監査"]:::world
        end

        %% 外部④：重い計算・ビルド処理
        subgraph HeavyCompute["④ 💻 client (VRAlchemy: Pure Rust) ＆ 🛡️ 盗品特徴量ワーカー (Optional)"]
            ClientBuilder["クライアント側ローカルLODビルダー (#![forbid(unsafe_code)])<br/>・🔥 重い3D変換・ポリゴン削減・KTX2圧縮はユーザーのPCパワーで実行！<br/>・(Full Hub構成時のみ) 別コンテナのRustワーカーで3D特徴量(pgvector)算出"]:::client
        end
    end

    %% ====================================================================================
    %% assets-server ⇔ 外部サーバー間の疎結合インターフェース
    %% ====================================================================================
    AuthIdentity -.->|"① JWKS公開鍵検証 ＆<br/>フィード閲覧時のフレンド/フォロー照会"| CommsPost & StorageVault
    BillingCore -.->|"② サブスク変更Webhook<br/>(quota_bytes 上限の更新のみ)"| StorageVault
    MarketEngine -.->|"③ 購入完了通知 / License Token<br/>(購入者へ quota_exempt:true のGrant付与<br/>＋ 出品者の該当Tagに削除ロック)"| BlobGrant
    Shopfront -.->|"④ 有料商品(:market_linked)の<br/>決済・価格管理を委譲"| MarketEngine
    CommsPost -.->|"⑤ 投稿・メール着信イベント発火"| WorldRealtime
    VcsCore -.->|"⑥ Origin Cert で自作権利を証明"| WorldRealtime
    ClientBuilder ===>|"⑦ ローカルビルド済みBlobのアップロード<br/>＆ 署名付きURL / P2Pでの高速取得"| AssetBlob
```

---

## 層の境界（draft）

| 層 | Domain（仮） | 置くもの | 置かないもの |
|:---|:---|:---|:---|
| 1 物理 CAS | （ストレージ） | `AssetBlob`（blake3 PK・size・mime・variants） | 所有者・ファイル名・価格・Quota |
| 2 Vault | `AlchemyAssets.Vault` | `StorageVault` / `BlobGrant`（権利・容量・削除ロック） | 決済実行・価格・License 署名本体 |
| 3 Drive | `AlchemyAssets.Drive` | `DriveNode` / `CommsPost`（階層・visibility） | トレンド SNS・開放リプライ・揮発チャット |
| 4 Vcs | `AlchemyAssets.Vcs` | Repo/Commit/Tree/Ref・Origin Cert | リアルタイム部屋状態・着用監査の実行 |
| 5 Shop | `AlchemyAssets.Shop` | Shopfront / ShopRelease・Tag ピン・distribution_mode | MoR/Stripe・エスクロー・連合検索 |

## 外部境界（draft）

| 境界 | 役割 | assets との接点 |
|:---|:---|:---|
| auth-server | JWT / JWKS / Friendship・Follow・Group・Block | Bearer 検証・フィード visibility 照会 |
| market + billing | 課金・コイン・License Token・連合マーケット | Vault の `quota_bytes` 更新、購入完了 → Grant |
| world + zenoh | Push・揮発チャット・音声・ワープ・verifier | 投稿イベント、Origin Cert / License 監査 |
| client (+ optional 特徴量) | LOD/KTX2 ローカルビルド、任意で pgvector | Blob アップロード / 署名 URL・P2P 取得 |

---

## GET / BlobGrant（目標メモ）

現行 GET 方針は正本に閉じた。目標モデルでは **GET 可否の SSoT は `BlobGrant`**（物理 `AssetBlob` は所有者を持たない）。

| grant_type（仮） | Quota | 典型用途 |
|:---|:---|:---|
| `:owned` | 消費する | 自作アップロード・Drive 保存 |
| `:purchased` | `quota_exempt` | market 購入完了通知で付与 |
| `:mail_temp` | `quota_exempt`（未 Claim） | Comms 添付の一時保持 |

段階導入（Grant 表 → owner/license 照合）はスライス化時に決める。**検証を外す／fail-open にはしない。**

---

## 未決・次の問い

- `BlobRef(owner_sub, blake3)` から `UNIQUE (vault_id, blob_hash)` / BlobGrant への移行単位
- `:free_direct` の GET を「Grant 必須」とするか「公開 Tag は読み取り capability のみ」とするか
- Stage 1 SQLite で Shop/Drive/Vcs を同じ TX に載せる範囲（最初は Vault + Blob のみか）
- 脅威モデル目標列（PoP / Immutable Tag / Safe HTTP 等）のスライス分割順 — [assets-threat-model.md](assets-threat-model.md)

採用して正本化するときは、本ファイルを昇格するか [assets-server.md](assets-server.md) へ取り込み、保証表とコードを一致させる。セキュリティ完了条件は脅威モデルの **目標の防御** を写す。
