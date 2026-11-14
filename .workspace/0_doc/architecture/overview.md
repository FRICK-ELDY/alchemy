# AlchemyEngine — アーキテクチャ概要（目標設計）

> **ステータス**: 目標アーキテクチャ

---

## 図の読み方（要約）

| ブロック | 役割 |
|:---|:---|
| **1. 外部決済・公開エッジ ＆ Open Zenoh Relay** | MoR/Stripe・Cloudflare（WAF/CDN/R2）・公式/サードパーティ relay-server・連合ノード |
| **2. 運営側コアインフラ** | 拠点 Proxmox 上の auth / assets / world(+verifier) / billing / market・DB・貸し出し host プール・NAS |
| **3. 共通契約層** | Protobuf ワイヤ契約・shared-sim / host-server・Dual Representation・Dual-AST Kernel（旧称 Formula VM） |
| **4. ホストユーザー端末** | client（親）・オンデマンド子プロセス host-server・OSC 受信・Dual Editor・LOD ビルド |
| **5. 参加プレイヤー ＆ マルチプラットフォーム** | Trusted メンバーの 1対1 P2P・外部ゲストのリレー経由・展開ロードマップ |

### 用語（Dual Representation / Dual-AST Kernel）

| 用語 | 指すもの | 旧称・備考 |
|:---|:---|:---|
| **Formula** | 空間ギミック用の DSL／ノード表面（ノードグラフ ⇄ テキスト） | Dual Representation の言語面。実行器ではない |
| **Dual Representation** | ノード＆テキスト両対応の共通規格（シェーダー＝WGSL 面、ギミック＝Formula 面） | どちらも単一の共通 AST へ変換 |
| **Dual-AST Kernel** | 共通 AST を評価する Pure Rust 実行器（目標・未実装） | 旧称 **Formula VM**。旧 Elixir Formula NIF とは別物 |
| **`FormulaTest`** | v1 第一級コンテンツの歴史名・識別子 | コンテンツ ID として非置換 |

通信フロー番号（①〜⑭）は図下部のエッジラベルに対応する。

---

## 全体構成図

```mermaid
flowchart TB
    %% ====================================================================================
    %% カラースタイル定義
    %% ====================================================================================
    classDef cf fill:#f48120,stroke:#ffd166,stroke-width:2px,color:#fff
    classDef vps fill:#005f73,stroke:#94d2bd,stroke-width:3px,color:#fff
    classDef relay3rd fill:#0a9396,stroke:#94d2bd,stroke-width:2px,stroke-dasharray: 5 5,color:#fff
    classDef net fill:#1f2937,stroke:#9ca3af,stroke-width:2px,color:#fff
    classDef elixir fill:#4e2a8e,stroke:#d0bfff,stroke-width:2px,color:#fff
    classDef guard fill:#1b4332,stroke:#74c69d,stroke-width:2px,color:#fff
    classDef vcs fill:#2d6a4f,stroke:#b7e4c7,stroke-width:2px,color:#fff
    classDef billing fill:#4361ee,stroke:#b8c0ff,stroke-width:2px,color:#fff
    classDef market fill:#9b2226,stroke:#f4a261,stroke-width:2px,color:#fff
    classDef storage fill:#2b2d42,stroke:#adb5bd,stroke-width:2px,color:#fff
    classDef host fill:#d97706,stroke:#fde68a,stroke-width:3px,color:#fff
    classDef rust fill:#b7410e,stroke:#ffd8a8,stroke-width:2px,color:#fff
    classDef shader fill:#800f2f,stroke:#ff8fa3,stroke-width:2px,color:#fff
    classDef launcher fill:#374151,stroke:#9ca3af,stroke-width:2px,color:#fff
    classDef local fill:#5f0f40,stroke:#ff70a6,stroke-width:2px,color:#fff
    classDef shared fill:#3c096c,stroke:#e0aaff,stroke-width:2px,stroke-dasharray: 5 5,color:#fff
    classDef member fill:#065f46,stroke:#6ee7b7,stroke-width:2px,color:#fff
    classDef guest fill:#1e3a8a,stroke:#93c5fd,stroke-width:2px,color:#fff
    classDef platform fill:#1d3557,stroke:#a8dadc,stroke-width:2px,color:#fff
    classDef evil fill:#370617,stroke:#dc2f02,stroke-width:2px,stroke-dasharray: 5 5,color:#ffba08

    %% ====================================================================================
    %% 1. 外部決済・クラウドエッジ ＆ オープン Zenoh リレー経済圏レイヤー
    %% ====================================================================================
    subgraph ExternalAndEdgeLayer["1. 外部決済・公開エッジ防波堤 ＆ Open Zenoh Relay 経済圏レイヤー (拠点IP完全秘匿)"]
        direction TB

        subgraph PaymentCloud["外部決済プロバイダ (MoR / Stripe API)"]
            StripeMoR["・MoR (Lemon Squeezy / Paddle 等) または Stripe Billing / Connect<br/>・プリペイドコイン決済 ＆ クリエイター/リレー貸し屋への報酬振込 (KYC連携)"]:::billing
        end

        subgraph CloudflareEdge["Cloudflare ネットワーク (WAF / DDoS防御 / SSL終端 / CDN)"]
            CFWAF["Cloudflare WAF & CDN Proxy<br/>全HTTPS API通信の窓口 (拠点ProxmoxのIPを秘匿)"]:::cf
            CFR2[("Cloudflare R2 (CAS選択肢①)<br/>・S3互換オブジェクトストレージ<br/>・下り通信量(Egress)完全無料<br/>・バンドル化でClass A/B API回数を極小化")]:::storage
        end

        subgraph OpenRelayNetwork["Open Zenoh Relay ネットワーク (マルチテナント相乗り ＆ サブスク 70:30 分配対象)"]
            direction LR
            OfficialVPS["公式リレー: ConoHa VPS (Ubuntu / 4Core / 4GB)<br/>【relay-server (zenohd)；Relay Agent は互換】<br/>・【現状】TLS :7447 待受 + mTLS / role CN ACL（DB非搭載・ステートレス）；短命 mTLS は拠点 world が署名（x0066；Agent はフォールバック）；Room Token は host-server（zenohd は JWT を見ない）<br/>・【目標】QUIC / WebTransport 待受 ／ 名前空間隔離 alchemy/tenant/{運営者ID}/room/{room_id}/** ／ Token Bucket の CCU・帯域 QoS<br/>・$0 無料枠ユーザーおよびサードパーティリレー障害時の緊急フォールバック先"]:::vps

            ThirdPartyRelay["サードパーティ『Zenoh リレー貸し屋』ノード群 (Open Zenoh Relay 規格)<br/>【インフラ提供者が自前VPSや10G回線で zenohd コンテナのみを起動 (DB不要)】<br/>・/.well-known/alchemy-relay-info.json 公開<br/>・個人運営者から指名されると、その運営者の月額サブスクの 30% を毎月獲得<br/>・market-server でイベント用の『VIP専用リレー貸切パス』も販売可能"]:::relay3rd

            FedNodeB["他運営者の連合ノード (Node B)<br/>・S2S HTTPS で World一覧 / JWKS / Marketカタログを相互交換<br/>・Trusted 設定時のみ ACL 制限付き Zenoh ブリッジ接続"]:::vps

            EvilMarketNode["外部の悪質マーケット (Blocked)<br/>・海賊版・盗品に偽ライセンスを発行<br/>➔ FederationPolicy で『Blocked』指定し即時遮断"]:::evil
        end
    end

    %% ====================================================================================
    %% 2. 運営側コアインフラ (拠点ネットワーク)
    %%     ※Stage 1(個人: SQLite単一構成) から Stage 4(大規模: PostgreSQLクラスタ) まで同一コードで伸縮
    %% ====================================================================================
    subgraph OperatorHomeInfra["2. 運営側コアインフラ (拠点 @nifty光 10GbE ➔ NTT 10G ONU ➔ YAMAHA RTX1300 [NAT 65,534セッション] ➔ 2.5G/10GbE LAN)"]
        direction TB

        subgraph ProxmoxServer["Proxmox VE サーバー (PC1: Ryzen 7 5800H [8C/16T] / 32GB RAM — Stage 1〜4 対応)"]
            direction TB

            %% --- CT1: 通信ブリッジ ---
            subgraph CT1_Bridge["CT 1: 通信ブリッジ層 (Ubuntu LXC / 1Core / 1GB)"]
                Cloudflared["cloudflared (Cloudflare Tunnel)<br/>RTX1300のポート開放ゼロで内側からHTTPS公開"]:::cf
                HomeZenoh["ローカル relay-server (zenohd)<br/>・拠点LAN内(作業用PC等)を 0.5ms で高速ルーティング<br/>・ConoHa VPS (または指名リレー屋) へ外向きブリッジ接続"]:::vps
            end

            %% --- CT2: Elixir コア & オプション群 ---
            subgraph CT2_ElixirCore["CT 2: Elixir コア ＆ 経済圏サーバー群 (Ubuntu LXC / 4Core / 8GB — Stage 1時は単一BEAMリリース同梱可)"]
                direction TB

                subgraph CoreProfile["【必須コア基盤 (Core Profile — 永続権威 ＆ セッション台帳の絶対的 SSoT)】"]
                    direction LR

                    AuthServer["auth-server (Elixir 1.19 / OTP 28 / Phoenix 1.8 / Ash 3.0)<br/>・RS256 署名 JWT 発行 (sub = user_id UUID)<br/>・Remember Me (7日スライディング Refresh Token)<br/>・利用規約同意管理 (tos_agreed / tos_agreed_at / tos_version)<br/>・Follow（X型）＋ Friendship（相互フォロー時のみ申請）SSoT / Group は後続<br/>・/.well-known/jwks.json 公開鍵配信"]:::elixir


                    AssetsServer["assets-server (Elixir / Ash + Git型 Spatial VCS + 容量管理)<br/>・Quota Enforcer (プラン別容量上限ゲート):<br/> $0: 1GB ｜ $1: 5GB ｜ $5: 25GB ｜ $10: 50GB ｜ $20: 100GB + グループ共有Vault 50GB<br/> (※マーケット購入品はリンク保持のみで容量消費0 / 改変Commit差分のみ計上)<br/>・インベントリ階層整理 ＆ WorldManifest / AlchemyPrefab 管理<br/>・Git操作: Commit / Branch / Tag / Diff / Fork<br/>・原産地証明 (Origin Cert: origin_id + 作者sub + CommitHash + 親ID) 電子署名<br/>・アップロード検疫: クライアントビルド済みGLB/KTX2の軽量ヘッダ検査<br/>・3D特徴量検疫 (pgvector): ボーン構造/UV形状/画像pHash類似度で頂点ずらし盗品を自動隔離"]:::vcs

                    WorldServer["world-server (100% Pure Elixir Umbrella — NIF完全廃止・案内所・門番・監査)<br/>・apps/core & network:<br/> - World一覧配信 ＆ S2S HTTPS による他運営者との連合リスト交換<br/> - /.well-known/alchemy-node-info.json (機能ディスカバリ配信)<br/> - alchemy:// ディープリンク解決 (URL 1つで特定座標へ直ワープ)<br/> - 7段階公開範囲 (LAN/Private/Friends/Friends+/Followers/Followers+/Public) ＆ 短命 Room Token 発行<br/>
 - 空間台帳 (Scene Manifest: 誰が何をスポーン・着用可能か) の署名発行<br/> - ※独自UDP (Network.UDP) と空間用Phoenix Channelは廃止し Zenoh に一本化<br/>・apps/contents:<br/> - 空間オーディオの『指揮者(Conductor)』として非同期コマンド発行<br/> - 旧v1第一級コンテンツ3本 (CanvasTest/BulletHell3D/FormulaTest) を検証用Manifestとして保持"]:::elixir


                    VerifierApp["world-server: apps/verifier (ゼロトラスト改ざん監査 ＆ スマート死活監視移譲)<br/>・【Untrustedホスト監査】ローカルhost-serverのRenderFrameを抜き打ち監査(Spot-Check)。<br/> 不正ワープや未許可アセット混入検知時は Room Token を即時失効しホスト権限を剥奪<br/>・【見る権利と着る権利の分離】表示は全員即時OK、着用・配置時のみ<br/> Origin Cert ➔ License Token ➔ 着用者JWT sub の署名チェーンを検証<br/>・【スマートホスト移譲】client_info の『引き受けON/OFF』と『PC性能(CPU/RAM/Ping)』を監視。<br/> ホスト終了/突然死時は『①予備ヘッドレスB/C』➔『②最高性能PCゲスト』へ0秒で昇格指令<br/>・【オープンリレーSLA監視】公認リレー屋のPing・稼働率(98%+)を検査し、障害時は公式リレーへ自動切替"]:::guard
                end

                subgraph OptionalProfile["【オプション拡張基盤 (Optional Profile — 個人運営ではOFF可能 / 外部サービスと連合可能)】"]
                    direction LR

                    BillingServer["BillingProvider (決済アダプタ ＆ 180日期限コインウォレット)<br/>・資金決済法(有効期限180日以内＝供託・届出除外) ＆ 特商法規約同意(tos_version)強制<br/>・少額決済($1プラン等)の手数料負けを防ぐプリペイドコインチャージ (譲渡・換金不可)<br/>・【サブスク自動スプリット (70 : 30 ルール)】:<br/> - 70% ➔ ストレージ ＆ コア運営維持費へ<br/> - 30% ➔ ユーザーが指名した『Open Zenoh Relay 貸し屋』の報酬枠へプール<br/>  (※通信量GB比例ではなく『指名スロット枠 × 稼働率SLA』で分配し水増し詐欺を防止)<br/>・Relay Allocation Token (RS256署名: sub / relay_id / max_ccu / max_mbps) 発行"]:::billing

                    MarketServer["market-server (Elixir / Phoenix / Ash — BOOTH風連合マーケット)<br/>・ワールド / アバター / シェーダー / ギミックを Git Tag単位で売買 ＋ 専用リレーパス販売<br/>・購入利用権証書 (License Token: Origin署名 + 購入者sub) をRS256で署名発行<br/>・出品者Trust Tier ＆ 新規売上14日間エスクロー (盗品発覚時は自動返金・ライセンス一斉失効・凍結)<br/>・マーケット連合管理 (FederationPolicy):<br/> - Trusted: 提携マーケットとカタログ相互検索・着用許可<br/> - Blocked: 悪質マーケットを即時遮断・ワールド内での着用を強制シルエット化"]:::market
                end

                WorldServer --> VerifierApp
                BillingServer <--> MarketServer
                BillingServer -.->|"Quota即時更新 (1GB〜100GB+Group50GB)"| AssetsServer
                MarketServer <-->|"Git Tag参照・権利連携"| AssetsServer
            end

            %% --- CT3: データベース ---
            subgraph CT3_Database["CT 3: データベース層 (Ubuntu LXC / 2Core / 4GB)"]
                UnifiedDB[("Ash DataLayer 統合DB<br/>・Stage 1 (個人開始時): SQLite (WALモード・ファイル1個)<br/>・Stage 2〜4 (規模拡大時): PostgreSQL 16+ + pgvector拡張<br/>(Users / ToS / Friends / Groups / Quotas / Commits / 3D指紋 / Licenses)")]:::storage
            end

            %% --- CT4: 常設・貸し出しヘッドレスプール ---
            subgraph CT4_HeadlessPool["CT 4: 常設・貸し出し host-server プール (Ubuntu LXC / 8Core / 16GB)"]
                CloudHosts["運営側 host-server インスタンス A / B / C (100% Pure Rust 冗長構成)<br/>・重たいワールドの貸し出し、24時間常設パブリック部屋、Quest単体主催時に稼働<br/>・【0人部屋オートスリープ (Wake-on-Join)】:<br/> 参加者0人でTick一時停止(CPU消費 0%) ➔ 1人目のRoom Token発行で0.01秒起床<br/>・ヘッドレスAが落ちても予備B・Cがスナップショットを引き継ぎ無停止フェイルオーバー"]:::host
            end

            Cloudflared --> AuthServer & AssetsServer & WorldServer & MarketServer
            AuthServer & AssetsServer & WorldServer & MarketServer --> UnifiedDB
            WorldServer & VerifierApp <===>|"監査・死活監視・起動制御"| HomeZenoh
            CloudHosts <===>|"30Hz(可変) RenderFrame"| HomeZenoh
        end

        %% --- マシン②：拠点 NAS ---
        subgraph HomeNAS["拠点 NAS: QNAP TS-632X-4G (CAS選択肢②：10GbE SFP+ 接続)"]
            QNAPStorage[("QuObjects (S3互換オブジェクトストレージ)<br/>・アセット不変バイナリ (GLB / KTX2 / Audio) 保管<br/>・Proxmox コンテナ ＆ DB の自動バックアップ (UPS連動保護)<br/>※assets-server の環境変数 (.env) 1行で Cloudflare R2 と自在に切替・併用可能")]:::storage
        end
    end

    %% ====================================================================================
    %% 3. 共通契約・同一バイナリ基盤レイヤー (SHARED CONTRACTS & CRATES)
    %% ====================================================================================
    subgraph SharedLayer["3. 共通契約・同一バイナリ基盤 (運営側・ホスト側・全クライアント環境で単一ソースを共有)"]
        direction LR

        ProtocolCrate["protocol (alchemy-protocol)<br/>・Protobuf (.proto) ワイヤ契約 SSoT (PROTOCOL_PIN で厳密固定)<br/>・render_frame.proto: 描画ワイヤ空間量は binary64 (f64) + デルタ(差分)フラグ<br/>・input_events.proto: 移動・アクション・空間編集入力 (+ Room Token)<br/>・client_info.proto: ホスト引き受け可否(can_be_host) / CPU余裕度 / 空きRAM / Ping RTT<br/>・audio & tracking: Opus音声フレーム / 圧縮済フルトラ・BlendShape配列"]:::shared

        SharedSimCrate["shared-sim / host-server (100% Pure Rust — Cargo Workspace 同居)<br/>・空間状態保持 (単精度 f32) + Rapier3d 物理エンジン + Dual-AST Kernel (Pure Rust)<br/>・【3層デカップリング・ティック構造】:<br/> ① 内部物理・Dual-AST Kernel Tick: 常時 60Hz 固定 (16.6ms — 壁抜け防止・関節安定化・通信費0円)<br/> ② ネットワーク配信 Tick: 可変 20Hz(省エネ) / 30Hz(標準) / 60Hz(高速) ＋ 静止物スリープ差分送信<br/> ③ クライアント予測補間 Tick: ~60Hz–120Hz<br/>・実行形態: PCでは『独立裏プロセス(host-server)』、Web/Mobile/Questでは『インプロセス(Worker)』"]:::shared

        DualRepSpec["Dual Representation (ノード ＆ テキスト両対応共通規格)<br/>・役割: Formula＝ギミック DSL 表面 / Dual-AST Kernel＝共通 AST 実行器（旧称 Formula VM）・NIF とは別<br/>・シェーダー: 3D空間ノードグラフ(DAG) ⇄ WGSL テキスト関数<br/>・空間ギミック: Formula ノードグラフ ⇄ Formula スクリプト<br/>・どちらも単一の共通 AST に変換 → Naga / Dual-AST Kernel で実行"]:::shader
    end

    %% ====================================================================================
    %% 4. クライアント側：ホストユーザー端末
    %% ====================================================================================
    subgraph HostUserPC["4. クライアント側：作業用・ホストPC (Windows / Ryzen 9 7950X [16C/32T] / 64GB RAM / RTX 4090)"]
        direction TB

        subgraph LocalHardwareEdge["手元の外部トラッカー・周辺機器 (localhost / 家庭内Wi-Fi)"]
            OSCDevices["・フルボディトラッカー (SlimeVR / mocopi / HaritoraX)<br/>・フェイシャル / アイトラッカー (VRCFaceTracking 等)<br/>※標準の OSC over UDP パケットを送信"]:::local
        end

        %% --- 親プロセス：client（Steam「プレイ」はここだけ） ---
        subgraph ClientProcess["親プロセス：client — VRAlchemy (100% Pure Rust ※リレー無しエンドポイント)"]
            direction TB

            ClientBootAndOSC["① 起動・VR自動判定・OSC 受信 (app / window / xr / input)<br/>・Steam/直起動は client のみ（全員 Client Only）。host-server はまだ起動しない<br/>・OpenXR あり ➔ VR / なし ➔ Desktop<br/>・OSC over UDP (rosc) ➔ is_finite() ➔ Protobuf ➔ Zenoh"]:::local

            HostSupervisor["①b HostSupervisor（薄い保証単位）<br/>・Join: host 未起動 / ソロ・ホスト開始: 子プロセスとして host-server をオンデマンド spawn<br/>・ready の ipc= で loopback IPC / Job Object(Win)・PDEATHSIG(Linux) でクラッシュ道連れ<br/>・終了: Handover stub → IPC SHUTDOWN（失敗時は部屋終了）。Exit Client only は廃止"]:::launcher

            ClientMenuUI["② 空間メニューパネル (system_ui)<br/>・World / Inventory / Git / Shop。ホスト引き受けトグル・貸し出しホスト / 公認リレー選択"]:::vcs

            ClientDualEditor["③ Dual Editor（ノード ＆ テキスト）"]:::shader

            ClientAssetAndCache["④ アセット LOD ビルダー ＆ セキュアキャッシュ"]:::guard

            ClientAVEngine["⑤ shared-sim / render / audio"]:::rust

            ClientBootAndOSC --> HostSupervisor
            ClientBootAndOSC & ClientMenuUI & ClientDualEditor & ClientAssetAndCache --> ClientAVEngine
        end

        %% --- 子プロセス：ローカル host-server（ホスト時のみ） ---
        subgraph LocalHostProcess["子プロセス：ローカル host-server (ホスト開始時のみ・Pure Rust ヘッドレス)"]
            LocalHostEngine["ローカル空間計算ホスト (Untrusted — verifier 監査対象)<br/>・Room Token / is_finite() / 60Hz 物理 / 30Hz 可変デルタ RenderFrame<br/>・音声・フルトラ・アセット本体は通さない"]:::host
        end

        OSCDevices ===>|"標準 OSC (UDP)"| ClientBootAndOSC
        HostSupervisor ===>|"オンデマンド spawn / IPC shutdown / Job Object"| LocalHostProcess
        LocalHostEngine <===>|"ローカル IPC (loopback)"| ClientAVEngine
    end

    %% ====================================================================================
    %% 5. クライアント側：ハイブリッドP2P負荷分散 ＆ ホスト引き継ぎ
    %%     鉄則：クライアントは絶対にサブ・リレー(他人の通信の中継役)にはならず、1対1 P2Pか専用リレー経由に限定
    %% ====================================================================================
    subgraph ParticipantsAndPlatforms["5. 参加プレイヤー群 (ハイブリッドP2P負荷分散 ＆ スマートホスト引き継ぎ) ＆ マルチプラットフォーム展開"]
        direction TB

        subgraph RoomParticipants["ワールド参加者 (例：登録メンバー10人 ＋ 外部ゲスト5人 ＝ 15人集会でもリレー負荷は数Mbps)"]
            direction LR

            TrustedMembers["自サーバー登録メンバー / 相互フレンド (Trusted Peers — 9人)<br/>・既定は client のみ。ホストを引き受けるときだけ子プロセス host-server を起動<br/>・【1対1 ダイレクトP2P (QUIC Hole Punching)】:<br/> 信頼関係がある仲間同士は、音声(Opus)・フルトラ表情・P2Pキャッシュを<br/> リレーを通さず1対1で直接送受信 (※他人の通信の中継＝サブリレー化は絶対しない)<br/>・【スマートホスト昇格待機】:<br/> client_info で『ホスト引き受けON』かつ高スペックなメンバーBは、<br/> ホストAが終了/突然死した瞬間に直前の受信フレームから0秒で新ホストへ自動昇格"]:::member

            ExternalGuests["外部ゲストアカウント (Untrusted / 非フレンド — 5人)<br/>・client (または Webブラウザ / Quest単体)<br/>・【IP秘匿リレー経由】:<br/> 見知らぬゲストに対してはお互いの拠点IPアドレスを隠すため、<br/> ダイレクトP2Pは使わず必ず『relay-server (ConoHa VPS / 公認リレー屋)』を経由<br/>・入室した瞬間に有料アバター含む全員の姿を R2 / リレーキャッシュから表示"]:::guest
        end

        subgraph TargetPlatforms["単一 Pure Rust コードベースからのマルチプラットフォーム展開ロードマップ (Step 1〜5)"]
            direction LR
            Step1["Step 1: Windows<br/>DX12/Vulkan + OpenXR<br/>(別プロセス host-server)"]:::platform
            Step2["Step 2: Linux / macOS<br/>Vulkan/Metal + OpenXR<br/>(別プロセス host-server)"]:::platform
            Step3["Step 3: Web (Browser)<br/>Wasm32 + WebGPU + WebXR<br/>(インプロセス Worker Host<br/>+ Zenoh over WebTransport)"]:::platform
            Step4["Step 4: Android / iOS<br/>Vulkan/Metal + Touch/AR<br/>(インプロセス Worker Host)"]:::platform
            Step5["Step 5: Quest等 Standalone VR<br/>Android Vulkan + OpenXR<br/>(インプロセス Worker Host<br/>+ 軽量KTX2/LOD自動取得)"]:::platform
            Step1 --> Step2 --> Step3 --> Step4 --> Step5
        end
    end

    %% ====================================================================================
    %% 全体通信・決済・監査・同期フロー結線
    %% ====================================================================================
    %% 1. 決済・サブスク70:30分配・リレーチケット交付
    StripeMoR <===>|"① コイン購入決済 / Webhook通知"| BillingServer
    StripeMoR ===>|"② 月末SLA達成確認後、サブスクの30%をリレー貸し屋へ・売上をクリエイターへ報酬振込"| ThirdPartyRelay & MarketServer

    %% 2. HTTPS API 通信 (Cloudflare Tunnel 経由で拠点IP完全秘匿)
    ClientMenuUI & TrustedMembers & ExternalGuests ===>|"③ HTTPS REST API (ログインJWT / 規約同意 / World一覧 / Room Token取得 / 買い物)"| CFWAF
    CFWAF ===>|"Cloudflare Tunnel (ポート開放不要)"| Cloudflared
    WorldServer <===>|"④ S2S HTTPS 連合 (World一覧・JWKS・Marketカタログ交換)"| FedNodeB
    MarketServer -.->|"ドメイン＆公開鍵を『Blocked』遮断"| EvilMarketNode

    %% 3. アセットLODビルド保存 ＆ R2 / NAS 配信 (ホストやリレーを一切圧迫しない直結経路)
    ClientAssetAndCache ===>|"⑤ 自PCでビルドしたPC版+Quest軽量LODをGit Commit (Quota検査)"| AssetsServer
    AssetsServer ===>|"⑥ S3互換APIで保存 (.env 1行でR2と拠点NASを切替)"| CFR2 & QNAPStorage
    CFR2 & QNAPStorage -.->|"⑦ 重い3D/画像バイナリはCDN/S3から直接高速DL (Egress 0円・ホスト負荷 0)"| ClientAssetAndCache & TrustedMembers & ExternalGuests

    %% 4. Zenoh 空間物理同期 (60Hz内部計算 ➔ 30Hz可変デルタ RenderFrame 配信)
    LocalHostEngine ===>|"⑧ 30Hz(20/30/60Hz可変) デルタ RenderFrame(binary64) を1回だけPublish"| HomeZenoh
    HomeZenoh <===>|"⑨ ブリッジ接続 (拠点から外向き接続)"| OfficialVPS & ThirdPartyRelay
    OfficialVPS & ThirdPartyRelay ===>|"⑩ 全参加者へ一斉配信 (Fan-out) ＆ verifier が抜き打ち監査"| TrustedMembers & ExternalGuests & VerifierApp

    %% 5. 音声 (Zenoh Audio Streaming)・フルトラ表情・キャッシュのハイブリッド経路 (host-server バイパス)
    ClientAVEngine <===>|"⑪ 仲間内(登録メンバー/フレンド)は1対1ダイレクトP2Pで音声・表情・キャッシュを直接交換 (リレー負荷 0)"| TrustedMembers
    ExternalGuests <===>|"⑫ 外部ゲスト分の音声(Opus over QUIC)・表情・入力のみリレーがIP秘匿中継"| OfficialVPS & ThirdPartyRelay

    %% 6. 入力・監査・スマート死活監視フェイルオーバー
    TrustedMembers & ExternalGuests ===>|"⑬ 移動・空間編集入力 (with Room Token)"| OfficialVPS & ThirdPartyRelay ===>|"入力転送"| LocalHostEngine & CloudHosts
    VerifierApp -.->|"⑭ ホスト終了/突然死(または改ざん剥奪)検知 ➔ 予備ヘッドレスB/C または 引き受けONの最高性能メンバーBへ0秒移行指令"| CloudHosts & TrustedMembers

    %% 7. 共通クレート層の参照リンク
    ProtocolCrate -.-> WorldServer & CloudHosts & LocalHostProcess & ClientProcess & RoomParticipants
    SharedSimCrate -.-> CloudHosts & LocalHostEngine & ClientAVEngine & TrustedMembers
    DualRepSpec -.-> AssetsServer & ClientDualEditor
```

---

## 関連ドキュメント

### HTTPS 入口（運用）

- Windows 手順書: [`manual/cloudflare-tunnel-windows.md`](../manual/cloudflare-tunnel-windows.md)
- Cloudflare Tunnel テンプレ / runbook: [`edge/cloudflared/`](../../../edge/cloudflared/)（Slice [x001a](../../roadmap/slices/x001a-https-edge-tunnel.md)）
- 外クライアント env 例: [`edge/client.env.example`](../../../edge/client.env.example)

### Zenoh WAN（運用）

- WAN 手順正本: [`manual/relay-server-wan.md`](../manual/relay-server-wan.md)（Slice [x001b](../../roadmap/slices/x001b-zenoh-wan-conoha.md)）
- DNS / Let's Encrypt（追加）: [`manual/relay-server-dns.md`](../manual/relay-server-dns.md)
- WAN config: [`relay-server/config.wan.json5`](../../../relay-server/config.wan.json5)

### Alchemy 側

- [auth-server](auth-server.md) · [assets-server](assets-server.md) · [world-server](world-server.md)
- [protocol](protocol.md) · [relay-server](relay-server.md)
- [trust-boundary](trust-boundary.md)（脅威モデル / 防御責任・x0031）
- [launcher](launcher.md)（**廃止**・HostSupervisor へ移譲）· [client](client.md)

### 転送と OSC

クライアント⇔サーバー／ホスト間の **Network Transport は Zenoh のみ**。OSC over UDP はクライアント局所の Device Input Edge（検証・Protobuf 後に Zenoh 送信）。
