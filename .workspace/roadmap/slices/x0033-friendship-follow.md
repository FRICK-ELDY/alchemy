# x0033 — Follow / Friendship（auth SSoT）

> **ID:** `x0033` · **slug:** `friendship-follow` · **status:** `done` · **area:** `[auth, docs]` · **depends_on:** `[x0032]`

目標: X型フォロー＋相互フォロー時のみフレンド申請。公開範囲7段階の正本を architecture に固定。

正本: [auth-server.md](../../0_doc/architecture/auth-server.md) · [overview.md](../../0_doc/architecture/overview.md)

## 設計

| 層 | 内容 |
|:---|:---|
| Follow | 有向 `follower → followee`。自由 follow/unfollow |
| Mutual | 双方向 Follow（派生） |
| Friend request | Mutual のときだけ。accept で accepted |
| Friends | unfriend で解除。片方が unfollow しても pending / accepted とも解除 |


## やらない

- Block / Group / JWT への関係載せ
- world Token 門番（→ x0034）
- 2hop Friends+ / Followers+（→ x0035）
- LAN 経路（→ x0036）

## 成功条件

1. Mutual なし friend_request → 422 `not_mutual`
2. accept 後 `friends?`
3. unfollow で pending / accepted とも解除（同一トランザクション）


4. architecture / overview に7段階が書かれている

← [目次](../README.md)
