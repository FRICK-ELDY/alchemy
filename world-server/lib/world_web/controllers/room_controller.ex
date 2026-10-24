defmodule WorldWeb.RoomController do
  use WorldWeb, :controller

  alias World.Rooms
  alias World.Token

  def create(conn, params) do
    room_id = Map.get(params, "room_id")

    case Rooms.create(room_id) do
      {:ok, id} ->
        conn
        |> put_status(:created)
        |> json(%{room_id: id})

      {:error, :already_exists} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "room_already_exists"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})
    end
  end

  def create_token(conn, %{"room_id" => room_id} = params) do
    role = Map.get(params, "role", "guest")

    case resolve_sub(conn, params) do
      {:ok, sub} ->
        issue_token(conn, room_id, role, sub)

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp resolve_sub(conn, params) do
    case conn.assigns[:auth_sub] do
      sub when is_binary(sub) and sub != "" ->
        {:ok, sub}

      _ ->
        if Application.get_env(:world, :allow_anon_tokens, false) do
          {:ok, Map.get(params, "sub", "anon")}
        else
          {:error, :anon_disabled}
        end
    end
  end

  defp issue_token(conn, room_id, role, sub) do
    with {:ok, _} <- Rooms.ensure(room_id),
         {:ok, token, ttl} <- Token.generate(room_id, role, sub) do
      json(conn, %{
        token: token,
        expires_in: ttl,
        room_id: room_id,
        role: role,
        sub: sub,
        token_type: "Bearer"
      })
    else
      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, :invalid_role} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_role"})

      {:error, reason} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "token_issue_failed", detail: inspect(reason)})
    end
  end
end
