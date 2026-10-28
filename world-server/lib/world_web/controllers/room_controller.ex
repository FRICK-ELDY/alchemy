defmodule WorldWeb.RoomController do
  use WorldWeb, :controller

  require Logger

  alias World.CanonicalManifest
  alias World.HostAttestation
  alias World.ManifestSeal
  alias World.RoomAccess
  alias World.Rooms
  alias World.Token

  def create(conn, params) do
    room_id = Map.get(params, "room_id")
    visibility = Map.get(params, "visibility", "public")

    case resolve_sub(conn, params) do
      {:ok, owner_sub} ->
        case Rooms.create(room_id, owner_sub, visibility) do
          {:ok, id, stored_visibility} ->
            conn
            |> put_status(:created)
            |> json(%{room_id: id, owner_sub: owner_sub, visibility: stored_visibility})

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

          {:error, :invalid_visibility} ->
            conn
            |> put_status(:unprocessable_entity)
            |> json(%{error: "invalid_visibility"})

          {:error, :storage_error} ->
            storage_unavailable(conn)
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

  def update_visibility(conn, %{"room_id" => room_id} = params) do
    visibility = Map.get(params, "visibility")

    case resolve_sub(conn, params) do
      {:ok, sub} ->
        room_id
        |> Rooms.set_visibility(sub, visibility || "")
        |> render_visibility_update(conn, room_id)

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp render_visibility_update({:ok, stored_visibility}, conn, room_id) do
    json(conn, %{room_id: room_id, visibility: stored_visibility})
  end

  defp render_visibility_update({:error, :not_found}, conn, _room_id) do
    conn
    |> put_status(:not_found)
    |> json(%{error: "room_not_found"})
  end

  defp render_visibility_update({:error, :forbidden_host}, conn, _room_id) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: "owner_only"})
  end

  defp render_visibility_update({:error, :invalid_visibility}, conn, _room_id) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_visibility"})
  end

  defp render_visibility_update({:error, :invalid_room_id}, conn, _room_id) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_room_id"})
  end

  defp render_visibility_update({:error, :storage_error}, conn, _room_id) do
    storage_unavailable(conn)
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
    with {:ok, room} <- Rooms.get(room_id),
         :ok <- RoomAccess.authorize(conn, room.owner_sub, sub, room.visibility),
         :ok <- authorize_role_cached(room, role, sub),
         {:ok, token, ttl} <- Token.generate(room_id, role, sub) do
      json(conn, %{
        token: token,
        expires_in: ttl,
        room_id: room_id,
        role: role,
        sub: sub,
        visibility: room.visibility,
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

      {:error, :forbidden_visibility} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "visibility_forbidden"})

      {:error, :lan_required} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "lan_required"})

      {:error, :auth_unavailable} ->
        conn
        |> put_status(:service_unavailable)
        |> json(%{error: "auth_access_unavailable"})

      {:error, :invalid_role} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_role"})

      {:error, :storage_error} ->
        storage_unavailable(conn)

      {:error, reason} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "token_issue_failed", detail: inspect(reason)})
    end
  end

  # x0021 P1-4: only the room owner may request role=host (room already loaded).
  defp authorize_role_cached(_room, "guest", _sub), do: :ok

  defp authorize_role_cached(%{owner_sub: owner}, "host", sub) when owner == sub, do: :ok

  defp authorize_role_cached(_room, "host", _sub), do: {:error, :forbidden_host}

  defp authorize_role_cached(_room, _role, _sub), do: {:error, :invalid_role}

  @doc """
  x0025: issue HostAttestation JWT. Authorization must be a **host** Room Token.
  """
  def create_host_key(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- Rooms.ensure_owner(room_id, claims["sub"]),
         {:ok, host_pub} <- HostAttestation.decode_host_pub(Map.get(params, "host_pub")),
         {:ok, host_generation} <- Rooms.claim_host_generation(room_id, host_pub) do
      case HostAttestation.generate(room_id, host_pub, claims["sub"], host_generation) do
        {:ok, attestation, ttl} ->
          json(conn, %{
            attestation: attestation,
            expires_in: ttl,
            room_id: room_id,
            host_generation: host_generation,
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

      {:error, :storage_error} ->
        storage_unavailable(conn)

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

      {:error, :storage_error} ->
        storage_unavailable(conn)

      {:error, reason} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token", detail: inspect(reason)})
    end
  end

  @doc """
  x0026 / x0039: issue ManifestSeal JWT for **canonical** DB scene body.

  Authorization must be a **host** Room Token. Body requires `host_attestation`
  plus host-computed `revision` / `manifest_blake3` for mismatch rejection
  (oracle abolished — world recomputes hash from scene + attestation).
  """
  def create_manifest_seal(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- Rooms.ensure_owner(room_id, claims["sub"]),
         {:ok, host_attestation} <- require_host_attestation(params),
         {:ok, host_revision} <- ManifestSeal.parse_revision(Map.get(params, "revision")),
         {:ok, host_blake3} <- ManifestSeal.parse_blake3(Map.get(params, "manifest_blake3")),
         {:ok, revision, host_generation, blake3} <-
           CanonicalManifest.canonical_blake3(room_id, host_attestation),
         :ok <- match_revision(host_revision, revision),
         :ok <- match_blake3(host_blake3, blake3) do
      case ManifestSeal.generate(room_id, revision, host_generation, blake3, claims["sub"]) do
        {:ok, seal, ttl} ->
          json(conn, %{
            seal: seal,
            expires_in: ttl,
            room_id: room_id,
            revision: revision,
            host_generation: host_generation,
            manifest_blake3: blake3,
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

      {:error, :revision_mismatch} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "revision_mismatch"})

      {:error, :invalid_manifest_blake3} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_manifest_blake3"})

      {:error, :manifest_blake3_mismatch} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "manifest_blake3_mismatch"})

      {:error, :missing_host_attestation} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "missing_host_attestation"})

      {:error, :invalid_host_attestation} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_host_attestation"})

      {:error, :attestation_room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "attestation_room_mismatch"})

      {:error, :stale_host_attestation} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "stale_host_attestation"})

      {:error, :invalid_scene_revision} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "invalid_scene_revision"})

      {:error, {:cli_missing, _} = reason} ->
        Logger.error("manifest-seal-hash missing: #{inspect(reason)}")

        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "manifest_seal_hash_unavailable"})

      {:error, {:cli_failed, _, _} = reason} ->
        Logger.error("manifest-seal-hash failed: #{inspect(reason)}")

        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "manifest_seal_hash_failed"})

      {:error, {:unexpected_hash_output, _} = reason} ->
        Logger.error("manifest-seal-hash bad output: #{inspect(reason)}")

        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "manifest_seal_hash_failed"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, :storage_error} ->
        storage_unavailable(conn)

      # Token.verify / auth failures only — CanonicalManifest/CLI errors must not land here.
      {:error, reason} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token", detail: inspect(reason)})
    end
  end

  defp require_host_attestation(%{"host_attestation" => token})
       when is_binary(token) and byte_size(token) > 0,
       do: {:ok, token}

  defp require_host_attestation(_), do: {:error, :missing_host_attestation}

  defp match_revision(host, canon) when host == canon, do: :ok
  defp match_revision(_, _), do: {:error, :revision_mismatch}

  defp match_blake3(host, canon) when host == canon, do: :ok
  defp match_blake3(_, _), do: {:error, :manifest_blake3_mismatch}

  defp fetch_bearer(conn) do
    case get_req_header(conn, "authorization") do
      ["Bearer " <> token] when byte_size(token) > 0 -> {:ok, token}
      _ -> {:error, :missing_bearer}
    end
  end

  # HTTP_* capabilities (scene / attest / seal) are defined in alchemy-protocol
  # `sim_capability` (x002a). Elixir cannot import that crate — keep `role == "host"`
  # here in lockstep with the protocol table.
  defp require_host_token(%{"role" => "host", "room_id" => token_room}, room_id) do
    if token_room == room_id do
      :ok
    else
      {:error, :room_mismatch}
    end
  end

  defp require_host_token(_, _), do: {:error, :not_host_token}

  defp storage_unavailable(conn) do
    conn
    |> put_status(:service_unavailable)
    |> json(%{error: "storage_unavailable"})
  end
end
