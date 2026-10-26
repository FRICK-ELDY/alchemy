defmodule Assets.CAS.Local do
  @moduledoc """
  Local disk CAS keyed by BLAKE3 hex filename.
  """

  @behaviour Assets.CAS.Backend

  @impl true
  def exists?(blake3_hex) when is_binary(blake3_hex) do
    File.exists?(path_for(blake3_hex))
  end

  @impl true
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

  @impl true
  def read(blake3_hex) when is_binary(blake3_hex) do
    case File.read(path_for(blake3_hex)) do
      {:ok, data} -> {:ok, data}
      {:error, :enoent} -> {:error, :not_found}
      {:error, reason} -> {:error, reason}
    end
  end

  @spec path_for(String.t()) :: String.t()
  def path_for(blake3_hex) when is_binary(blake3_hex) do
    Path.join(cas_root(), blake3_hex)
  end

  defp cas_root, do: Application.fetch_env!(:assets, :cas_path)
end
