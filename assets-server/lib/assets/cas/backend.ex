defmodule Assets.CAS.Backend do
  @moduledoc """
  Content-addressed blob storage backend (local disk or S3-compatible).
  """

  @callback exists?(blake3_hex :: String.t()) :: boolean()
  @callback put_if_absent(blake3_hex :: String.t(), data :: binary()) :: :ok | {:error, term()}
  @callback read(blake3_hex :: String.t()) :: {:ok, binary()} | {:error, :not_found | term()}
end
