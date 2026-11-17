defmodule AssetsWeb.VcsController do
  @moduledoc """
  HTTP negotiate / push for Spatial VCS (x00cd / draft §7.1·§18.6).

  Guarantees: Bearer JWT via `:user_auth`; owner-only; same semantics as
  `Assets.Vcs.negotiate/3` / `push/3`. Push object bodies are base64 (`bytes_b64`).
  Blobs remain `POST /api/v1/blobs`.

  Non-guarantees: clone/fetch; SSH; Save UI.
  """

  use AssetsWeb, :controller

  alias Assets.Vcs
  alias Assets.Vcs.Ref, as: VcsRef

  # Negotiate payloads are hash lists only.
  @negotiate_body_limit 65_536
  # Push may carry several ≤1 MiB objects as base64 (~4/3 expansion).
  @push_body_limit 12_582_912

  def negotiate(conn, %{"repo_id" => repo_id}) do
    with {:ok, body, conn} <- read_json_body(conn, @negotiate_body_limit),
         {:ok, attrs} <- negotiate_attrs(body) do
      case Vcs.negotiate(repo_id, conn.assigns.auth_sub, attrs) do
        {:ok, result} ->
          json(conn, %{
            missing_object_hashes: result.missing_object_hashes,
            missing_blob_blake3s: result.missing_blob_blake3s
          })

        {:error, reason} ->
          respond_vcs_error(conn, reason)
      end
    else
      {:error, :invalid_json} ->
        conn |> put_status(:bad_request) |> json(%{error: "invalid_json"})

      {:error, :too_large} ->
        conn |> put_status(413) |> json(%{error: "too_large"})

      {:error, reason} ->
        conn |> put_status(:unprocessable_entity) |> json(%{error: error_code(reason)})
    end
  end

  def push(conn, %{"repo_id" => repo_id}) do
    with {:ok, body, conn} <- read_json_body(conn, @push_body_limit),
         {:ok, attrs} <- push_attrs(body) do
      case Vcs.push(repo_id, conn.assigns.auth_sub, attrs) do
        {:ok, %VcsRef{} = vref} ->
          json(conn, %{ref: ref_json(vref)})

        {:error, reason} ->
          respond_vcs_error(conn, reason)
      end
    else
      {:error, :invalid_json} ->
        conn |> put_status(:bad_request) |> json(%{error: "invalid_json"})

      {:error, :too_large} ->
        conn |> put_status(413) |> json(%{error: "too_large"})

      {:error, reason} ->
        conn |> put_status(:unprocessable_entity) |> json(%{error: error_code(reason)})
    end
  end

  defp negotiate_attrs(body) when is_map(body) do
    with {:ok, ref_name} <- fetch_binary(body, "ref_name"),
         {:ok, new_hash} <- fetch_hash(body, "new_commit_hash"),
         {:ok, old_hash} <- optional_old(body),
         {:ok, object_hashes} <- fetch_hash_list(body, "object_hashes"),
         {:ok, blob_blake3s} <- fetch_hash_list(body, "blob_blake3s") do
      {:ok,
       %{
         ref_name: ref_name,
         new_commit_hash: new_hash,
         old_commit_hash: old_hash,
         object_hashes: object_hashes,
         blob_blake3s: blob_blake3s
       }}
    end
  end

  defp push_attrs(body) when is_map(body) do
    with {:ok, ref_name} <- fetch_binary(body, "ref_name"),
         {:ok, new_hash} <- fetch_hash(body, "new_commit_hash"),
         {:ok, old_hash} <- optional_old(body),
         {:ok, objects} <- fetch_objects(body) do
      {:ok,
       %{
         ref_name: ref_name,
         new_commit_hash: new_hash,
         old_commit_hash: old_hash,
         objects: objects
       }}
    end
  end

  defp optional_old(body) do
    case Map.get(body, "old_commit_hash") do
      nil -> {:ok, nil}
      hash when is_binary(hash) and byte_size(hash) == 64 -> {:ok, hash}
      "" -> {:ok, nil}
      _ -> {:error, {:invalid_attr, "old_commit_hash"}}
    end
  end

  defp fetch_binary(body, key) do
    case Map.get(body, key) do
      v when is_binary(v) and byte_size(v) > 0 -> {:ok, v}
      _ -> {:error, {:invalid_attr, key}}
    end
  end

  defp fetch_hash(body, key) do
    case Map.get(body, key) do
      v when is_binary(v) and byte_size(v) == 64 -> {:ok, v}
      _ -> {:error, {:invalid_attr, key}}
    end
  end

  defp fetch_hash_list(body, key) do
    case Map.get(body, key) do
      list when is_list(list) ->
        if Enum.all?(list, &(is_binary(&1) and byte_size(&1) == 64)) do
          {:ok, list}
        else
          {:error, {:invalid_attr, key}}
        end

      _ ->
        {:error, {:invalid_attr, key}}
    end
  end

  defp fetch_objects(%{"objects" => list}) when is_list(list) do
    list
    |> Enum.reduce_while({:ok, []}, fn item, {:ok, acc} ->
      case decode_object(item) do
        {:ok, obj} -> {:cont, {:ok, [obj | acc]}}
        {:error, _} = err -> {:halt, err}
      end
    end)
    |> case do
      {:ok, objs} -> {:ok, Enum.reverse(objs)}
      err -> err
    end
  end

  defp fetch_objects(_), do: {:error, {:invalid_attr, "objects"}}

  defp decode_object(%{"hash_hex" => hash, "bytes_b64" => b64})
       when is_binary(hash) and byte_size(hash) == 64 and is_binary(b64) do
    case Base.decode64(b64) do
      {:ok, bytes} -> {:ok, %{hash_hex: hash, bytes: bytes}}
      :error -> {:error, :invalid_object_bytes}
    end
  end

  defp decode_object(_), do: {:error, :invalid_object}

  defp read_json_body(conn, limit) do
    case Plug.Conn.read_body(conn, length: limit) do
      {:ok, body, conn} ->
        case Jason.decode(body) do
          {:ok, map} when is_map(map) -> {:ok, map, conn}
          _ -> {:error, :invalid_json}
        end

      {:more, _, _conn} ->
        {:error, :too_large}

      {:error, _} ->
        {:error, :invalid_json}
    end
  end

  defp respond_vcs_error(conn, reason) do
    {status, code} = vcs_status(reason)
    conn |> put_status(status) |> json(%{error: code})
  end

  defp vcs_status(:forbidden), do: {:forbidden, "forbidden"}
  defp vcs_status(:not_found), do: {:not_found, "not_found"}
  defp vcs_status(:ref_protected), do: {:forbidden, "ref_protected"}
  defp vcs_status(:no_grant), do: {:forbidden, "no_grant"}
  defp vcs_status(:cas_mismatch), do: {:conflict, "cas_mismatch"}
  defp vcs_status(:not_fast_forward), do: {:conflict, "not_fast_forward"}
  defp vcs_status(:frozen), do: {:conflict, "frozen"}
  defp vcs_status(:author_mismatch), do: {:unprocessable_entity, "author_mismatch"}
  defp vcs_status(:too_many_commits), do: {:unprocessable_entity, "too_many_commits"}
  defp vcs_status(:missing_tip_commit), do: {:unprocessable_entity, "missing_tip_commit"}
  defp vcs_status({:missing_blob, _}), do: {:unprocessable_entity, "missing_blob"}
  defp vcs_status({:unreachable_commit, _}), do: {:unprocessable_entity, "unreachable_commit"}
  defp vcs_status({:missing_tree, _}), do: {:unprocessable_entity, "missing_tree"}
  defp vcs_status({:missing_commit, _}), do: {:unprocessable_entity, "missing_commit"}
  defp vcs_status(reason), do: {:unprocessable_entity, error_code(reason)}

  defp error_code({:invalid_attr, key}) when is_binary(key), do: "invalid_attr:#{key}"
  defp error_code(atom) when is_atom(atom), do: Atom.to_string(atom)
  defp error_code({atom, _}) when is_atom(atom), do: Atom.to_string(atom)
  defp error_code({atom, _, _}) when is_atom(atom), do: Atom.to_string(atom)
  defp error_code(_), do: "invalid_request"

  defp ref_json(%VcsRef{} = vref) do
    %{
      ref_id: vref.ref_id,
      repo_id: vref.repo_id,
      name: vref.name,
      kind: vref.kind,
      commit_hash: vref.commit_hash,
      protected: vref.protected,
      is_frozen: vref.is_frozen
    }
  end
end
