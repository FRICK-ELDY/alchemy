defmodule WorldWeb.RoomController do
  use WorldWeb, :controller

  require Logger

  alias World.HostAttestation
  alias World.ManifestSeal
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

  @doc """
  x0025: issue HostAttestation JWT. Authorization must be a **host** Room Token.
  """
  def create_host_key(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- Rooms.require_existing(room_id),
         :ok <- require_owner(room_id, claims["sub"]),
         {:ok, host_pub} <- HostAttestation.decode_host_pub(Map.get(params, "host_pub")) do
      case HostAttestation.generate(room_id, host_pub, claims["sub"]) do
        {:ok, attestation, ttl} ->
          json(conn, %{
            attestation: attestation,
            expires_in: ttl,
            room_id: room_id,
            token_type: "Bearer"
          })

        {:error, reason} ->
          Logger.error("HostAttestation.generate failed: #{inspect(reason)}")

          conn
          |> put_status(:internal_server_error)
          |> json(%{error: "attestation_issue_failed", detail: inspect(reason)})
      end
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :not_host_token} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_required"})

      {:error, :room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "room_id_mismatch"})

      {:error, :forbidden_host} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_owner_only"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :invalid_host_pub} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_host_pub"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, reason} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token", detail: inspect(reason)})
    end
  end

  @doc """
  x0027: return world-authored scene definition. Authorization must be a **host** Room Token.
  """
  def show_scene(conn, %{"room_id" => room_id}) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         {:ok, scene} <- Rooms.scene(room_id) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :not_host_token} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_required"})

      {:error, :room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "room_id_mismatch"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, reason} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token", detail: inspect(reason)})
    end
  end

  @doc """
  x0026: issue ManifestSeal JWT. Authorization must be a **host** Room Token.
  """
  def create_manifest_seal(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- Rooms.require_existing(room_id),
         :ok <- require_owner(room_id, claims["sub"]),
         {:ok, revision} <- ManifestSeal.parse_revision(Map.get(params, "revision")),
         {:ok, blake3} <- ManifestSeal.parse_blake3(Map.get(params, "manifest_blake3")) do
      case ManifestSeal.generate(room_id, revision, blake3, claims["sub"]) do
        {:ok, seal, ttl} ->
          json(conn, %{
            seal: seal,
            expires_in: ttl,
            room_id: room_id,
            token_type: "Bearer"
          })

        {:error, reason} ->
          Logger.error("ManifestSeal.generate failed: #{inspect(reason)}")

          conn
          |> put_status(:internal_server_error)
          |> json(%{error: "manifest_seal_issue_failed", detail: inspect(reason)})
      end
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :not_host_token} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_required"})

      {:error, :room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "room_id_mismatch"})

      {:error, :forbidden_host} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "host_role_owner_only"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :invalid_revision} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_revision"})

      {:error, :invalid_manifest_blake3} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_manifest_blake3"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, reason} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token", detail: inspect(reason)})
    end
  end

  defp fetch_bearer(conn) do
    case get_req_header(conn, "authorization") do
      ["Bearer " <> token] when byte_size(token) > 0 -> {:ok, token}
      _ -> {:error, :missing_bearer}
    end
  end

  defp require_host_token(%{"role" => "host", "room_id" => token_room}, room_id) do
    if token_room == room_id do
      :ok
    else
      {:error, :room_mismatch}
    end
  end

  defp require_host_token(_, _), do: {:error, :not_host_token}

  defp require_owner(room_id, sub) do
    if Rooms.owner?(room_id, sub) do
      :ok
    else
      {:error, :forbidden_host}
    end
  end
end
