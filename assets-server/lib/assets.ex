defmodule Assets do
  @moduledoc """
  assets-server — CAS + Quota（x0003）+ object/ref（x003f）+ BlobGrant ledger（x004f）+
  inventory PUT grant_required（x0054）+ Personal Vault view（x0058）+
  Drive folder/blob/prefab（x005c/x005d）+ Prefab.Payload schema（x005e）+
  Drive take-out helpers（x005f）+ Drive put-back/save_prefab（x0060）+
  Vcs promote/commit/ref/cert（x0062–x0065）+ ItemRef blake3|repo@rev（x0063）+
  PlaceRead（x006b）+ GET option B（x006c）+ Drive put_back HTTP（x006e）.

  ## Guarantees

  - auth-gated PUT/GET, CasObject + per-owner BlobRef + owned BlobGrant
  - reserved→put→used; failure releases reservation only (no CAS.delete)
  - GET always BlobGrant or covering PlaceRead (x006c option B); `access` is PUT
    metadata / first-writer only — Bearer alone is never a read capability
  - HTTP inventory PUT (`POST /api/v1/inventory/blobs`) always grant_required (x0054)
  - authenticated CasObject may coexist with later grant_required PUT; CasObject.access
    stays first-writer
  - `Assets.Vault.personal/1` thin view over QuotaLedger; vault_id = owner_sub (x0058)
  - BLAKE3 CAS (local|s3), fixed quota
  - DriveNode CRUD (folder/blob/prefab/vcs_repo): owner-only, BlobGrant required for
    blob/prefab; circular parent rejected (x005c/x005d)
  - Prefab.Payload validates schema_version=1, children blake3 (64-char hex), finite-float
    TRS (non-finite NaN/Inf rejected) (x005e)
  - Drive.resolve_for_take_out/2: BlobGrant verified before returning refs to host (x005f)
  - Drive.save_prefab/4: Payload validated + uploaded grant_required + DriveNode upserted
    (x0060; invariant: save→reload→from_json round-trip is lossless — x0061)
  - HTTP `POST /api/v1/drive/put_back` wraps save_prefab for Desktop Inventory (x006e)
  - Vcs.promote_prefab/2: DriveNode(prefab)→VcsRepo + initial Commit in one TX;
    Commit.tree_json ≡ Prefab payload; old blake3 path remains readable (x0062)
  - ItemRef.resolve/2: {:blake3, hash} and {:repo_rev, repo_id, rev} paths;
    owner / BlobGrant checked; invalid rev → :not_found (x0063)
  - Vcs.create_branch/3, create_tag/3, advance_branch/3: Ref CRUD; tags immutable (frozen);
    advance_branch checks fast-forward reachability (x0064)
  - Vcs.issue_cert/3 / verify_cert/1: Ed25519 Origin Cert over canonical message;
    private key never logged or returned; tampered cert → :tampered (x0065)

  ## Non-guarantees

  - CasObject.access column removal / access→Grant peel (P1-1 remainder)
  - Group Vault/purchased/mail_temp, room-scoped grants, signed direct PUT, LOD
  - PlaceRead TTL refresh UI; client mesh render; existence oracle (403 vs 404)
  - Group Vault, sharing, Mail/Comms, Shopfront
  - VcsRepo fork/transfer, CA chain, cert revocation, License Token
  - CRDT, Formula VM, non-FF merge, 3D Diff
  """
end
