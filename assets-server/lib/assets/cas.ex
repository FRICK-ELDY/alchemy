defmodule Assets.CAS do
  @moduledoc """
  Content-addressed storage facade. Backend is `:local` (disk) or `:s3` (R2 / S3-compatible).
  """

  alias Assets.CAS.{Local, S3}

  @spec exists?(String.t()) :: boolean()
  def exists?(blake3_hex), do: backend().exists?(blake3_hex)

  @spec put_if_absent(String.t(), binary()) :: :ok | {:error, term()}
  def put_if_absent(blake3_hex, data), do: backend().put_if_absent(blake3_hex, data)

  @spec read(String.t()) :: {:ok, binary()} | {:error, :not_found | term()}
  def read(blake3_hex), do: backend().read(blake3_hex)

  defp backend do
    case Application.fetch_env!(:assets, :cas_backend) do
      :local -> Local
      :s3 -> S3
      other -> raise "unknown :cas_backend #{inspect(other)} (expected :local or :s3)"
    end
  end
end
