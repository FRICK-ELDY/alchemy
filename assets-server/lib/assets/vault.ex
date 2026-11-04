defmodule Assets.Vault do
  @moduledoc """
  Personal StorageVault thin view over QuotaLedger (x0058).

  Guarantees: `personal/1` returns used/reserved/limit matching `Assets.Blobs.quota/1`;
  provisional vault id key = `owner_sub` (documented; future UUID migration is a
  separate slice).
  Non-guarantees: Group Vault, BlobRef UNIQUE(vault_id, blob_hash), billing plans,
  `:read_only`, DriveNode / Shop.
  """

  alias Assets.Blobs

  @type vault_id :: String.t()
  @type personal_view :: %{
          vault_id: vault_id(),
          kind: :personal,
          used_bytes: non_neg_integer(),
          reserved_bytes: non_neg_integer(),
          limit_bytes: pos_integer()
        }

  @doc """
  Provisional Personal vault id. Wave 1 uses `owner_sub` as the stable key.
  """
  @spec personal_id(String.t()) :: vault_id()
  def personal_id(owner_sub) when is_binary(owner_sub), do: owner_sub

  @doc """
  Read-only Personal vault view. Bytes match `Assets.Blobs.quota/1`.
  """
  @spec personal(String.t()) :: personal_view()
  def personal(owner_sub) when is_binary(owner_sub) do
    q = Blobs.quota(owner_sub)

    %{
      vault_id: personal_id(owner_sub),
      kind: :personal,
      used_bytes: q.used_bytes,
      reserved_bytes: q.reserved_bytes,
      limit_bytes: q.limit_bytes
    }
  end
end
