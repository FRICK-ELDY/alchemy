defmodule Assets do
  @moduledoc """
  assets-server — CAS + Quota（x0003）+ object/ref 分離（x003f）.

  Guarantees: auth-gated put/get, CasObject + per-owner BlobRef, reserved→put→used,
  BLAKE3 CAS (local|s3), fixed quota.
  Non-guarantees: BlobGrant/Vault/Commit, signed direct PUT, LOD, client mesh render.
  """
end
