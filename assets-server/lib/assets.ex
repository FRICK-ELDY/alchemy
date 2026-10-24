defmodule Assets do
  @moduledoc """
  assets-server — Slice C-A local CAS + quota.

  Guarantees: auth-gated blob put/get, BLAKE3 CAS on disk, SQLite metadata, fixed quota.
  Non-guarantees: Vault/Commit VCS, R2/NAS, LOD, client mesh render.
  """
end
