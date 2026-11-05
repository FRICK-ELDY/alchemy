defmodule Assets.CAS.Backend do
  @moduledoc """
  Content-addressed blob storage backend (local disk or S3-compatible).
  """

  @type exists_result :: true | false | {:error, term()}

  @callback exists?(blake3_hex :: String.t()) :: exists_result()

  @callback put_if_absent(blake3_hex :: String.t(), data :: binary()) ::
              :created | :existing | {:error, term()}

  @callback read(blake3_hex :: String.t()) :: {:ok, binary()} | {:error, :not_found | term()}
  @callback delete(blake3_hex :: String.t()) :: :ok | {:error, term()}

  @doc """
  Move live object aside for orphan GC re-check (x0074).

  Returns `:ok` when quarantined, `{:error, :not_found}` when absent.
  """
  @callback quarantine(blake3_hex :: String.t()) :: :ok | {:error, :not_found | term()}

  @doc """
  Restore quarantined bytes to the live key, or drop quarantine if live already exists.
  """
  @callback restore_quarantine(blake3_hex :: String.t()) :: :ok | {:error, term()}

  @doc "Permanently delete quarantined bytes."
  @callback discard_quarantine(blake3_hex :: String.t()) :: :ok | {:error, term()}
end
