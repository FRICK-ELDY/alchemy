defmodule AssetsWeb.DriveController do
  @moduledoc """
  Personal Drive HTTP surface (x006d).

  Guarantees: owner-only list + take_out wrapping `Assets.Drive`.
  Non-guarantees: put-back, Prefab multi-child expand, folder UX.
  """

  use AssetsWeb, :controller

  alias Assets.Drive

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
            blobs: Enum.map(blobs, fn %{blake3: b} -> %{blake3: b} end)
          })

        {:error, :not_found} ->
          conn
          |> put_status(:not_found)
          |> json(%{error: "not_found"})

        {:error, :no_grant} ->
          conn
          |> put_status(:forbidden)
          |> json(%{error: "no_grant"})
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

  defp read_json_body(conn) do
    case Plug.Conn.read_body(conn, length: 4096) do
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

  defp node_json(node) do
    %{
      node_id: node.node_id,
      kind: node.kind,
      name: node.name,
      parent_id: node.parent_id,
      blake3: node.blake3
    }
  end
end
