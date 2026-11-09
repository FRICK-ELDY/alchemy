defmodule AssetsWeb.DriveController do
  @moduledoc """
  Personal Drive HTTP surface (x006d / x006e / x007f).

  Guarantees: owner-only list + take_out + put_back wrapping `Assets.Drive`.
  Prefab take_out expands Payload children (blake3 + local_id + optional trs).
  Non-guarantees: folder / vcs_repo take-out UX; host multi-child spawn.
  """

  use AssetsWeb, :controller

  require Logger

  alias Assets.Drive
  alias Assets.Prefab.Payload

  # Single-child put-back is tiny; headroom for a few TRS children without Plug.Parsers.
  @json_body_limit 16_384

  def index(conn, params) do
    parent_id =
      case Map.get(params, "parent_id") do
        id when is_binary(id) and id != "" -> id
        _ -> nil
      end

    nodes =
      Drive.list_children(conn.assigns.auth_sub, parent_id)
      |> Enum.map(&node_json/1)

    json(conn, %{nodes: nodes})
  end

  def take_out(conn, _params) do
    # Endpoint has no Plug.Parsers (raw blob PUT). Decode a small JSON body here.
    with {:ok, body, conn} <- read_json_body(conn),
         {:ok, node_id} <- fetch_node_id(body) do
      case Drive.resolve_for_take_out(conn.assigns.auth_sub, node_id) do
        {:ok, %{node: node, blobs: blobs}} ->
          json(conn, %{
            node: node_json(node),
            blobs: Enum.map(blobs, &blob_ref_json/1)
          })

        {:error, :not_found} ->
          conn
          |> put_status(:not_found)
          |> json(%{error: "not_found"})

        {:error, :no_grant} ->
          conn
          |> put_status(:forbidden)
          |> json(%{error: "no_grant"})

        {:error, :unsupported_kind} ->
          conn
          |> put_status(:unprocessable_entity)
          |> json(%{error: "unsupported_kind"})

        {:error, :invalid_prefab} ->
          conn
          |> put_status(:unprocessable_entity)
          |> json(%{error: "invalid_prefab"})

        {:error, :cas_missing} ->
          conn
          |> put_status(:not_found)
          |> json(%{error: "cas_missing"})
      end
    else
      {:error, :node_id_required} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "node_id_required"})

      {:error, :invalid_json} ->
        conn
        |> put_status(:bad_request)
        |> json(%{error: "invalid_json"})
    end
  end

  def put_back(conn, _params) do
    with {:ok, body, conn} <- read_json_body(conn),
         {:ok, name} <- fetch_name(body),
         {:ok, payload_map} <- fetch_payload_map(body),
         {:ok, payload} <- Payload.validate(payload_map),
         parent_id <- optional_parent_id(body) do
      case Drive.save_prefab(conn.assigns.auth_sub, name, payload, parent_id) do
        {:ok, node} ->
          json(conn, %{node: node_json(node)})

        {:error, :forbidden} ->
          conn
          |> put_status(:forbidden)
          |> json(%{error: "forbidden"})

        {:error, :parent_not_folder} ->
          conn
          |> put_status(:unprocessable_entity)
          |> json(%{error: "parent_not_folder"})

        {:error, reason} ->
          Logger.warning("drive put_back save_failed: #{inspect(reason)}")

          conn
          |> put_status(:unprocessable_entity)
          |> json(%{error: "save_failed"})
      end
    else
      {:error, :name_required} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "name_required"})

      {:error, :name_too_long} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "name_too_long"})

      {:error, :payload_required} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "payload_required"})

      {:error, :invalid_json} ->
        conn
        |> put_status(:bad_request)
        |> json(%{error: "invalid_json"})

      {:error, reason} ->
        Logger.warning("drive put_back invalid_payload: #{inspect(reason)}")

        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_payload"})
    end
  end

  defp read_json_body(conn) do
    case Plug.Conn.read_body(conn, length: @json_body_limit) do
      {:ok, body, conn} ->
        case Jason.decode(body) do
          {:ok, map} when is_map(map) -> {:ok, map, conn}
          _ -> {:error, :invalid_json}
        end

      {:more, _, _conn} ->
        {:error, :invalid_json}

      {:error, _} ->
        {:error, :invalid_json}
    end
  end

  defp fetch_node_id(%{"node_id" => id}) when is_binary(id) and id != "", do: {:ok, id}
  defp fetch_node_id(_), do: {:error, :node_id_required}

  defp fetch_name(%{"name" => name}) when is_binary(name) do
    trimmed = String.trim(name)

    cond do
      trimmed == "" -> {:error, :name_required}
      byte_size(trimmed) > 255 -> {:error, :name_too_long}
      true -> {:ok, trimmed}
    end
  end

  defp fetch_name(_), do: {:error, :name_required}

  defp fetch_payload_map(%{"payload" => payload}) when is_map(payload), do: {:ok, payload}
  defp fetch_payload_map(_), do: {:error, :payload_required}

  defp optional_parent_id(%{"parent_id" => id}) when is_binary(id) and id != "", do: id
  defp optional_parent_id(_), do: nil

  defp node_json(node) do
    %{
      node_id: node.node_id,
      kind: node.kind,
      name: node.name,
      parent_id: node.parent_id,
      blake3: node.blake3
    }
  end

  defp blob_ref_json(%{blake3: b3} = blob) do
    base = %{blake3: b3}

    base =
      case Map.get(blob, :local_id) do
        id when is_binary(id) -> Map.put(base, :local_id, id)
        _ -> base
      end

    case Map.get(blob, :trs) do
      %{} = trs -> Map.put(base, :trs, trs)
      _ -> base
    end
  end
end
