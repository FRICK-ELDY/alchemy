defmodule Assets.VcsObject do
  @moduledoc """
  Spatial VCS Tree/Commit codec via `vcs-object --serve` Port pool (x00c9).

  Guarantees: encode/verify share protocol golden codec; soft Port errors as
  `{:error, reason}` (fail-closed for promote).
  Non-guarantees: assets DB writes (caller); Cert v2; push.
  """

  alias Assets.VcsObject.Worker

  @spec encode_tree(map()) :: {:ok, %{bytes: binary(), hash_hex: String.t()}} | {:error, term()}
  def encode_tree(%{} = tree_fields) do
    body = Map.put(tree_fields, "op", "encode_tree")

    with {:ok, resp} <- request(body),
         {:ok, bytes} <- decode_b64(resp["bytes_b64"]),
         hash when is_binary(hash) <- resp["hash_hex"] do
      {:ok, %{bytes: bytes, hash_hex: hash}}
    else
      {:error, _} = err -> err
      _ -> {:error, :invalid_port_response}
    end
  end

  @spec encode_commit(map()) :: {:ok, %{bytes: binary(), hash_hex: String.t()}} | {:error, term()}
  def encode_commit(%{} = commit_fields) do
    body = Map.put(commit_fields, "op", "encode_commit")

    with {:ok, resp} <- request(body),
         {:ok, bytes} <- decode_b64(resp["bytes_b64"]),
         hash when is_binary(hash) <- resp["hash_hex"] do
      {:ok, %{bytes: bytes, hash_hex: hash}}
    else
      {:error, _} = err -> err
      _ -> {:error, :invalid_port_response}
    end
  end

  @spec verify(binary()) :: {:ok, map()} | {:error, term()}
  def verify(bytes) when is_binary(bytes) do
    body = %{"op" => "verify", "bytes_b64" => Base.encode64(bytes)}

    case request(body) do
      {:ok, resp} -> {:ok, resp}
      {:error, _} = err -> err
    end
  end

  defp request(map) do
    json = Jason.encode!(map)

    case Worker.call_json(json) do
      {:ok, %{"ok" => true} = resp} -> {:ok, resp}
      {:ok, other} -> {:error, {:unexpected_response, other}}
      {:error, reason} -> {:error, reason}
    end
  end

  defp decode_b64(nil), do: {:error, :missing_bytes_b64}

  defp decode_b64(b64) when is_binary(b64) do
    case Base.decode64(b64) do
      {:ok, bin} -> {:ok, bin}
      :error -> {:error, :bad_base64}
    end
  end
end
