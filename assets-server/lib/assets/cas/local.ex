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
      :existing
    else
      write_new_object(path, data)
    end
  end

  defp write_new_object(path, data) do
    tmp = path <> ".tmp-#{System.unique_integer([:positive])}"

    case File.write(tmp, data) do
      :ok -> rename_or_existing(tmp, path)
      {:error, reason} -> cleanup_tmp_or_existing(tmp, path, reason)
    end
  end

  defp rename_or_existing(tmp, path) do
    case File.rename(tmp, path) do
      :ok ->
        :created

      {:error, reason} ->
        # Peer won the race (common on Windows when target already exists).
        cleanup_tmp_or_existing(tmp, path, reason)
    end
  end

  defp cleanup_tmp_or_existing(tmp, path, reason) do
    _ = File.rm(tmp)
    if File.exists?(path), do: :existing, else: {:error, reason}
  end

  @impl true
  def read(blake3_hex) when is_binary(blake3_hex) do
    case File.read(path_for(blake3_hex)) do
      {:ok, data} -> {:ok, data}
      {:error, :enoent} -> {:error, :not_found}
      {:error, reason} -> {:error, reason}
    end
  end

  @impl true
  def delete(blake3_hex) when is_binary(blake3_hex) do
    case File.rm(path_for(blake3_hex)) do
      :ok -> :ok
      {:error, :enoent} -> :ok
      {:error, reason} -> {:error, reason}
    end
  end

  @spec path_for(String.t()) :: String.t()
  def path_for(blake3_hex) when is_binary(blake3_hex) do
    Path.join(cas_root(), blake3_hex)
  end

  defp cas_root, do: Application.fetch_env!(:assets, :cas_path)
end
