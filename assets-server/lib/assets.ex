defmodule Assets do
  @moduledoc """
  assets-server — CAS + Quota（x0003）+ object/ref（x003f）+ BlobGrant ledger（x004f）.

  Guarantees: auth-gated put/get, CasObject + per-owner BlobRef + owned BlobGrant,
  reserved→put→used, access=authenticated (default capability) or grant_required,
  BLAKE3 CAS (local|s3), fixed quota.
  Non-guarantees: GET always grant-gated (option B), Vault/purchased/mail_temp,
  room-scoped grants, signed direct PUT, LOD, client mesh render.
  """
end
