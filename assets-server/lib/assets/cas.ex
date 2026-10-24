defmodule Assets.CAS do
  @moduledoc """
  Local content-addressed storage keyed by BLAKE3 hex.
  """

  @spec path_for(String.t()) :: String.t()
  def path_for(blake3_hex) when is_binary(blake3_hex) do
    Path.join(cas_root(), blake3_hex)
  end

  @spec exists?(String.t()) :: boolean()
  def exists?(blake3_hex), do: File.exists?(path_for(blake3_hex))

  @spec put_if_absent(String.t(), binary()) :: :ok | {:error, term()}
  def put_if_absent(blake3_hex, data) when is_binary(data) do
    path = path_for(blake3_hex)
    File.mkdir_p!(Path.dirname(path))

    if File.exists?(path) do
      :ok
    else
      tmp = path <> ".tmp-#{System.unique_integer([:positive])}"

      with :ok <- File.write(tmp, data),
           :ok <- File.rename(tmp, path) do
        :ok
      else
        error ->
          _ = File.rm(tmp)
          error
      end
    end
  end

  @spec read(String.t()) :: {:ok, binary()} | {:error, :not_found | term()}
  def read(blake3_hex) do
    path = path_for(blake3_hex)

    case File.read(path) do
      {:ok, data} -> {:ok, data}
      {:error, :enoent} -> {:error, :not_found}
      {:error, reason} -> {:error, reason}
    end
  end

  defp cas_root, do: Application.fetch_env!(:assets, :cas_path)
end
