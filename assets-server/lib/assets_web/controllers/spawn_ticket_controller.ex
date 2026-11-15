defmodule AssetsWeb.SpawnTicketController do
  @moduledoc """
  Standalone SpawnTicket issue API for Grant holders (x00b5).

  Guarantees: Grant check per blake3 before issue; same claims as take_out bundling.
  Non-guarantees: world consume; rate-limit beyond Plug stack.
  """

  use AssetsWeb, :controller

  alias Assets.SpawnTicket

  @json_body_limit 16_384

  def create(conn, _params) do
    with {:ok, body, conn} <- read_json_body(conn),
         {:ok, room_id} <- fetch_room_id(body),
         {:ok, blake3s} <- fetch_blake3s(body) do
      case SpawnTicket.issue_for_grantee(conn.assigns.auth_sub, room_id, blake3s) do
        {:ok, token, ttl} ->
          json(conn, %{spawn_ticket: token, expires_in: ttl})

        {:error, :no_grant} ->
          conn |> put_status(:forbidden) |> json(%{error: "no_grant"})

        {:error, :invalid_room_id} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_room_id"})

        {:error, :blake3s_required} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "blake3s_required"})

        {:error, :blake3s_exceeded} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "blake3s_exceeded"})

        {:error, :invalid_blake3} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_blake3"})

        {:error, :invalid_args} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_args"})

        {:error, _reason} ->
          conn |> put_status(:unprocessable_entity) |> json(%{error: "issue_failed"})
      end
    else
      {:error, :room_id_required} ->
        conn |> put_status(:unprocessable_entity) |> json(%{error: "room_id_required"})

      {:error, :blake3s_required} ->
        conn |> put_status(:unprocessable_entity) |> json(%{error: "blake3s_required"})

      {:error, :invalid_json} ->
        conn |> put_status(:bad_request) |> json(%{error: "invalid_json"})
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

  defp fetch_room_id(%{"room_id" => id}) when is_binary(id) and id != "", do: {:ok, id}
  defp fetch_room_id(_), do: {:error, :room_id_required}

  defp fetch_blake3s(%{"blake3s" => list}) when is_list(list) do
    if list == [], do: {:error, :blake3s_required}, else: {:ok, list}
  end

  defp fetch_blake3s(%{"blake3" => b3}) when is_binary(b3) and b3 != "", do: {:ok, [b3]}
  defp fetch_blake3s(_), do: {:error, :blake3s_required}
end
