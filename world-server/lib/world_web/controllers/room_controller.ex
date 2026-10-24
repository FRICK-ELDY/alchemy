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
    if Application.get_env(:world, :allow_anon_tokens, false) do
      issue_anon_token(conn, room_id, params)
    else
      conn
      |> put_status(:unauthorized)
      |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp issue_anon_token(conn, room_id, params) do
    role = Map.get(params, "role", "guest")
    sub = Map.get(params, "sub", "anon")

    with {:ok, _} <- Rooms.ensure(room_id),
         {:ok, token, ttl} <- Token.generate(room_id, role, sub) do
      json(conn, %{
        token: token,
        expires_in: ttl,
        room_id: room_id,
        role: role,
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
