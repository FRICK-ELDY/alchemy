defmodule Assets do
  @moduledoc """
  assets-server — CAS + Quota（x0003）+ object/ref（x003f）+ BlobGrant ledger（x004f）+
  inventory PUT path（x0054）+ Personal Vault view（x0058）+
  Drive folder/blob/prefab（x005c/x005d）+ Prefab.Payload schema（x005e）+
  Drive take-out helpers（x005f）+ Drive put-back/save_prefab（x0060）+
  Vcs promote/commit/ref/cert（x0062–x0065）+ ItemRef blake3|repo@rev（x0063）+
  PlaceRead（x006b）+ GET option B（x006c）+ Drive put_back HTTP（x006e）+
  CasObject access column removed（x006f）+ reserved reaper / orphan CAS GC（x0074）.

  ## Guarantees

  - auth-gated PUT/GET, CasObject + per-owner BlobRef + owned BlobGrant
  - reserved→put→used; failure releases reservation only (no CAS.delete on compensate)
  - TTL `quota_reservations` reaper + grace orphan CAS GC (x0074; GenServer, not Oban)
  - GET always BlobGrant, covering PlaceRead (x006c), or RepoRead membership
    in a readable repo's `vcs_repo_blobs` (x00cb); CasObject is bytes meta
    only (no `access` column — x006f). Bearer alone is never a read capability
  - Local blob GET is always `application/octet-stream` + `X-Content-Type-Options:
    nosniff` (x008b); CasObject.mime is storage meta / upload echo only
  - PUT always issues `:owned` Grant; same blake3 may be claimed via `/blobs` and
    `/inventory/blobs` by different owners without access conflict (x006f)
  - HTTP inventory PUT (`POST /api/v1/inventory/blobs`) remains a path alias (x0054)
  - `Assets.Vault.personal/1` thin view over QuotaLedger; vault_id = owner_sub (x0058)
  - BLAKE3 CAS (local|s3), fixed quota
  - DriveNode CRUD (folder/blob/prefab/vcs_repo): owner-only, BlobGrant required for
    blob/prefab; circular parent rejected (x005c/x005d)
  - Prefab.Payload validates schema_version=1, children blake3 (64-char hex), finite-float
    TRS (non-finite NaN/Inf rejected) (x005e)
  - Drive.resolve_for_take_out/2: blob → single blake3; prefab → child mesh expand with
    Grant on payload + every child (x005f / x007f)
  - Drive.save_prefab/4 / create_prefab_node/4: every child BlobGrant required before
    upload / link (x00b4); missing → `:no_grant` without Quota/CAS spend; take_out
    still re-checks. save→reload→from_json round-trip lossless (x0060 / x0061)
  - HTTP `POST /api/v1/drive/put_back` wraps save_prefab; child Grant miss → 403
    `no_grant` (x006e / x00b4)
  - Vcs.promote_prefab/2: DriveNode(prefab)→VcsRepo + CBOR Tree/Commit + `main`
    in one TX; child Grants re-checked (x00b4); commit_hash = blake3(Commit bytes)
    via vcs-object Port (x0062 / x00c9); Authorship cert + `vcs_blob_origins`
    for child meshes (x00ca); `visibility=private` + `vcs_repo_blobs` index
    (x00cb); old prefab blake3 path remains readable
  - Vcs.set_visibility/3: owner flips private|public (x00cb); no fork detach /
    :purchased check yet
  - ItemRef.resolve/2: {:blake3, hash} needs BlobGrant; {:repo_rev, …} needs
    RepoRead (`can_read`) + children ∈ `vcs_repo_blobs` (x00cb); private
    non-owner → :not_found; missing index → :incomplete_index (not :no_grant);
    invalid rev → :not_found (x0063)
  - Vcs.create_branch/3, create_tag/3, advance_branch/3: Ref CRUD; tags immutable (frozen);
    advance_branch checks fast-forward reachability (x0064) and `protected`
    (x00cc); create_tag issues Release cert (subject = repo owner_sub) (x00ca)
  - Vcs.negotiate/3 + push/3: owner-only; tip-reachable only; Port verify; author
    match; Grant+CAS (prepare + TX recheck); ref CAS+FF; joining≤256; Authorship
    + repo_blobs + origins in one TX; protected → :ref_protected (x00cc).
  - Vcs.set_branch_protection/4: owner toggles branch `protected` (x00cc)
  - Vcs.issue_authorship/2 / issue_release/4 / verify_cert/1: Ed25519 Origin Cert v2
    (`v2|commit|…|issued_at_ms|kid` / `v2|release|…`); offline-verifiable;
    private key never logged or returned; tampered → :tampered (x00ca)

  ## Non-guarantees

  - Group Vault/purchased/mail_temp, room-scoped grants, signed direct PUT, LOD
  - PlaceRead TTL refresh UI; client mesh render; existence oracle (403 vs 404)
  - Group Vault, sharing, Mail/Comms, Shopfront
  - VcsRepo fork/transfer/HTTP VCS / client vcs-store, CA chain, cert revocation,
    License Token, `:purchased` push rejection
  - unauthenticated public repo browse; blob GET denial as 404
  - CRDT, Dual-AST Kernel, non-FF merge, 3D Diff
  - mime/size first-writer change; CasObject-with-zero-refs GC; S3 bucket listing scan
  """
end
