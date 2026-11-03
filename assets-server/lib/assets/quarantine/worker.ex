defmodule Assets.Quarantine.Worker do
  @moduledoc """
  Behaviour for asset quarantine / perceptual inspection.

  Guarantees: OSS trees expose only this callback surface for injection;
  the default impl is header-only or no-op.
  Non-guarantees: 3D perceptual-hash / pgvector similarity, stolen-asset
  heuristics, or any proprietary threshold — those live in private plugins
  (`LicenseRef-Alchemy-Proprietary`) configured via
  `:assets, :quarantine_worker`.
  """

  @type blake3_hex :: String.t()
  @type mime :: String.t()

  @typedoc "Outcome of a quarantine inspect pass."
  @type result :: :ok | {:quarantine, reason :: term()} | {:error, term()}

  @callback inspect_blob(blake3_hex(), data :: binary(), mime()) :: result()
end
