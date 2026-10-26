defmodule WorldWeb.RoomController do
  use WorldWeb, :controller

  alias World.Rooms
  alias World.Token

  def create(conn, params) do
    room_id = Map.get(params, "room_id")

    case resolve_sub(conn, params) do
      {:ok, owner_sub} ->
        case Rooms.create(room_id, owner_sub) do
          {:ok, id} ->
            conn
            |> put_status(:created)
            |> json(%{room_id: id, owner_sub: owner_sub})

          {:error, :already_exists} ->
            conn
            |> put_status(:conflict)
            |> json(%{error: "room_already_exists"})

          {:error, :missing_owner} ->
            conn
            |> put_status(:unprocessable_entity)
            |> json(%{error: "missing_owner"})

          {:error, :invalid_room_id} ->
            conn
            |> put_status(:unprocessable_entity)
            |> json(%{error: "invalid_room_id"})
        end

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
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

  # Token paths never create rooms (x0021): owner is only set by POST /rooms.
  defp issue_token(conn, room_id, role, sub) do
    with :ok <- Rooms.require_existing(room_id),
         :ok <- authorize_role(room_id, role, sub),
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

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :forbidden_host} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_owner_only"})

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

  # x0021 P1-4: only the room owner may request role=host.
  defp authorize_role(_room_id, "guest", _sub), do: :ok

  defp authorize_role(room_id, "host", sub) do
    if Rooms.owner?(room_id, sub) do
      :ok
    else
      {:error, :forbidden_host}
    end
  end

  defp authorize_role(_room_id, _role, _sub), do: {:error, :invalid_role}
end
