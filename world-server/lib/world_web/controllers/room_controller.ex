defmodule WorldWeb.RoomController do
  use WorldWeb, :controller

  require Logger

  alias World.AllocationToken
  alias World.CanonicalManifest
  alias World.HostAttestation
  alias World.HttpPop
  alias World.ManifestSeal
  alias World.MtlsIssuer
  alias World.PlaceHashes
  alias World.PlaceRead
  alias World.RoomAccess
  alias World.Rooms
  alias World.Rooms.RoomKeyGrace
  alias World.Token

  # x007c: catalog of rooms visible to the caller (no secrets).
  def index(conn, params) do
    case resolve_sub(conn, params) do
      {:ok, viewer_sub} ->
        mine_only? = mine_only_param?(params)

        case Rooms.list_visible_summaries(conn, viewer_sub, mine_only: mine_only?) do
          {:ok, rooms} ->
            json(conn, %{rooms: rooms})

          {:error, :storage_error} ->
            storage_unavailable(conn)
        end

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp mine_only_param?(params) do
    case Map.get(params, "mine") do
      v when v in ["1", "true", "TRUE", "yes", "YES"] -> true
      _ -> false
    end
  end

  # x0080: single-room session summary (no secrets) for Waiting / nomination polls.
  def show(conn, %{"room_id" => room_id} = params) do
    case resolve_sub(conn, params) do
      {:ok, viewer_sub} ->
        with {:ok, room} <- Rooms.get_session(room_id),
             :ok <- RoomAccess.authorize(conn, room.owner_sub, viewer_sub, room.visibility) do
          json(conn, room_session_summary(room, viewer_sub))
        else
          {:error, :not_found} ->
            conn |> put_status(:not_found) |> json(%{error: "room_not_found"})

          {:error, :invalid_room_id} ->
            conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_room_id"})

          {:error, :storage_error} ->
            storage_unavailable(conn)

          {:error, _} ->
            conn |> put_status(:forbidden) |> json(%{error: "visibility_forbidden"})
        end

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  def create(conn, params) do
    room_id = Map.get(params, "room_id")
    visibility = Map.get(params, "visibility", "public")

    case resolve_sub(conn, params) do
      {:ok, owner_sub} ->
        room_id
        |> Rooms.create(owner_sub, visibility)
        |> render_create(conn, owner_sub)

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp render_create({:ok, id, stored_visibility, wire_cap}, conn, owner_sub) do
    conn
    |> put_status(:created)
    |> json(%{
      room_id: id,
      owner_sub: owner_sub,
      visibility: stored_visibility,
      wire_cap: wire_cap
    })
  end

  defp render_create({:error, :already_exists}, conn, _owner_sub) do
    conn
    |> put_status(:conflict)
    |> json(%{error: "room_already_exists"})
  end

  defp render_create({:error, :owner_room_cap_exceeded}, conn, _owner_sub) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: "room_cap_exceeded"})
  end

  defp render_create({:error, :missing_owner}, conn, _owner_sub) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "missing_owner"})
  end

  defp render_create({:error, :invalid_room_id}, conn, _owner_sub) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_room_id"})
  end

  defp render_create({:error, :invalid_visibility}, conn, _owner_sub) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_visibility"})
  end

  defp render_create({:error, :storage_error}, conn, _owner_sub) do
    storage_unavailable(conn)
  end

  # Typical ECDSA/RSA CSR PEM is a few KB; cap rejects disk/openssl DoS bodies.
  @csr_pem_max_bytes 16_384

  def create_token(conn, %{"room_id" => room_id} = params) do
    role = Map.get(params, "role", "guest")

    case optional_csr_pem(params) do
      :too_large ->
        conn
        |> put_status(:request_entity_too_large)
        |> json(%{error: "csr_pem_too_large"})

      csr_pem ->
        with {:ok, sub} <- resolve_sub(conn, params),
             {:ok, session_pub} <- decode_token_session_pub(params) do
          issue_token(conn, room_id, role, sub, session_pub, csr_pem)
        else
          {:error, :anon_disabled} ->
            conn
            |> put_status(:unauthorized)
            |> json(%{error: "anon_tokens_disabled"})

          {:error, :invalid_session_pub} ->
            conn
            |> put_status(:unprocessable_entity)
            |> json(%{error: "invalid_session_pub"})
        end
    end
  end

  defp optional_csr_pem(params) do
    case Map.get(params, "csr_pem") do
      pem when is_binary(pem) and pem != "" and byte_size(pem) <= @csr_pem_max_bytes ->
        pem

      pem when is_binary(pem) and byte_size(pem) > @csr_pem_max_bytes ->
        :too_large

      _ ->
        nil
    end
  end

  defp decode_token_session_pub(params) do
    case Map.get(params, "session_pub") do
      pub when is_binary(pub) and pub != "" -> Token.decode_session_pub(pub)
      _ -> {:error, :invalid_session_pub}
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
  # x003e: session_pub (32B) is required and embedded as cnf.spk.
  # x004e: also returns wire_cap + Allocation Token for Relay Agent mTLS.
  # x0066: optional csr_pem → mtls client cert (world-issued; CSR Subject ignored).
  # x0069: room_key (URL-safe base64 of 32B) for Zenoh payload AEAD — not in Allocation JWT.
  # x007e: room_key_epoch accompanies room_key (Allocation JWT still has neither).
  # x0080: host role gated by active_host_sub / owner-resume; successful host issue activates lease.
  defp issue_token(conn, room_id, role, sub, session_pub, csr_pem) do
    relay_id = Application.fetch_env!(:world, :default_relay_id)

    with {:ok, room} <- Rooms.get(room_id),
         :ok <- RoomAccess.authorize(conn, room.owner_sub, sub, room.visibility),
         :ok <- authorize_role_cached(room, role, sub),
         {:ok, room} <- maybe_activate_host(room, role, sub),
         {:ok, room_key_b64} <- Rooms.room_key_for_token(room.room_key),
         {:ok, token, room_jti, ttl} <- Token.generate(room_id, role, sub, session_pub),
         {:ok, allocation_token, alloc_jti, _} <-
           AllocationToken.generate(room_id, role, sub, room.wire_cap, relay_id, room_jti),
         {:ok, mtls} <- maybe_issue_mtls(csr_pem, role) do
      Logger.metadata(jti: room_jti)

      Logger.info(
        "room token issued room_jti=#{room_jti} alloc_jti=#{alloc_jti} sub=#{sub} room_id=#{room_id} room_key_epoch=#{room.room_key_epoch} state=#{room.state}"
      )

      World.Metrics.inc(:token_issue_total)

      body = %{
        token: token,
        expires_in: ttl,
        room_id: room_id,
        role: role,
        sub: sub,
        visibility: room.visibility,
        wire_cap: room.wire_cap,
        room_key: room_key_b64,
        room_key_epoch: room.room_key_epoch,
        allocation_token: allocation_token,
        token_type: "Bearer",
        state: room.state,
        active_host_sub: room.active_host_sub
      }

      # x007e: host Token may carry RoomKeyGrace prev for dual-open after restart / late sync.
      # Guests never receive prev (no client multi-key buffer).
      body =
        if role == "host" do
          maybe_put_room_key_prev(body, room_id)
        else
          body
        end

      body =
        if mtls do
          Map.put(body, :mtls, %{
            client_cert_pem: mtls.client_cert_pem,
            ca_cert_pem: mtls.ca_cert_pem,
            expires_at: DateTime.to_iso8601(mtls.expires_at)
          })
        else
          body
        end

      json(conn, body)
    else
      reason ->
        if count_token_issue_error?(reason) do
          World.Metrics.inc(:token_issue_errors_total)
        end

        issue_token_error(conn, reason)
    end
  end

  # Server-side issue failures only — client authz / validation rejects stay out of the counter.
  defp count_token_issue_error?({:error, reason})
       when reason in [
              :openssl_failed,
              :sign_failed,
              :chmod_failed,
              :invalid_room_key,
              :storage_error,
              :auth_unavailable
            ],
       do: true

  defp count_token_issue_error?({:error, reason})
       when reason in [
              :not_found,
              :forbidden_host,
              :conflict,
              :forbidden_visibility,
              :lan_required,
              :invalid_role,
              :invalid_room_id,
              :invalid_args,
              :invalid_allocation_args
            ],
       do: false

  defp count_token_issue_error?({:error, _}), do: true
  defp count_token_issue_error?(_), do: true

  defp issue_token_error(conn, {:error, :openssl_failed}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "mtls_sign_failed"})
  end

  defp issue_token_error(conn, {:error, :sign_failed}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "mtls_sign_failed"})
  end

  defp issue_token_error(conn, {:error, :chmod_failed}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "mtls_sign_failed"})
  end

  defp issue_token_error(conn, {:error, :invalid_allocation_args}) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_allocation_args"})
  end

  defp issue_token_error(conn, {:error, :invalid_args}) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_csr_pem"})
  end

  defp issue_token_error(conn, {:error, :invalid_room_id}) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_room_id"})
  end

  defp issue_token_error(conn, {:error, :invalid_room_key}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "invalid_room_key"})
  end

  defp issue_token_error(conn, {:error, :not_found}) do
    conn
    |> put_status(:not_found)
    |> json(%{error: "room_not_found"})
  end

  defp issue_token_error(conn, {:error, :forbidden_host}) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: "host_role_forbidden"})
  end

  defp issue_token_error(conn, {:error, :conflict}) do
    conn
    |> put_status(:conflict)
    |> json(%{error: "host_claim_conflict"})
  end

  defp issue_token_error(conn, {:error, :forbidden_visibility}) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: "visibility_forbidden"})
  end

  defp issue_token_error(conn, {:error, :lan_required}) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: "lan_required"})
  end

  defp issue_token_error(conn, {:error, :auth_unavailable}) do
    conn
    |> put_status(:service_unavailable)
    |> json(%{error: "auth_access_unavailable"})
  end

  defp issue_token_error(conn, {:error, :invalid_role}) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_role"})
  end

  defp issue_token_error(conn, {:error, :storage_error}) do
    storage_unavailable(conn)
  end

  defp issue_token_error(conn, {:error, reason}) do
    Logger.error("Room Token issue failed: #{inspect(reason)}")

    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "token_issue_failed"})
  end

  defp issue_token_error(conn, reason) do
    Logger.error("Room Token issue failed: #{inspect(reason)}")

    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "token_issue_failed"})
  end

  # x0066: csr_pem present → sign; absent → no mtls field (Relay Agent path may still apply).
  # CA 未設定 / 読めない: csr を無視して Token を返す（Agent 併存）。
  # CA 設定あり・署名失敗（openssl）: Token ごと 5xx（WAN で world 発行を選んだ経路の fail-closed）。
  # x007e: attach in-grace prev key for host dual-open (guest Tokens omit).
  defp maybe_put_room_key_prev(body, room_id) when is_map(body) and is_binary(room_id) do
    case RoomKeyGrace.get(room_id) do
      {:ok, %{prev_key: hex, prev_epoch: epoch}} ->
        case Rooms.room_key_for_token(hex) do
          {:ok, b64} ->
            body
            |> Map.put(:room_key_prev, b64)
            |> Map.put(:room_key_prev_epoch, epoch)

          {:error, _} ->
            body
        end

      :miss ->
        body
    end
  end

  defp maybe_put_room_key_prev(body, _), do: body

  defp maybe_issue_mtls(nil, _role), do: {:ok, nil}
  defp maybe_issue_mtls("", _role), do: {:ok, nil}

  defp maybe_issue_mtls(csr_pem, role) when is_binary(csr_pem) do
    if MtlsIssuer.configured?() do
      case MtlsIssuer.issue(csr_pem, role) do
        {:ok, mtls} ->
          {:ok, mtls}

        {:error, :ca_not_configured} ->
          {:ok, nil}

        {:error, :ca_unreadable} ->
          Logger.warning("csr_pem ignored: MTLS CA unreadable (x0066 Agent coexistence)")
          {:ok, nil}

        {:error, other} ->
          {:error, other}
      end
    else
      Logger.info("csr_pem ignored: MTLS CA not configured (x0066 coexistence)")
      {:ok, nil}
    end
  end

  # x0080: host role = active_host_sub (or owner resume from dormant), not owner alone.
  defp authorize_role_cached(_room, "guest", _sub), do: :ok

  defp authorize_role_cached(%{} = room, "host", sub), do: Rooms.authorize_host_role(room, sub)

  defp authorize_role_cached(_room, _role, _sub), do: {:error, :invalid_role}

  defp maybe_activate_host(room, "host", sub) do
    Rooms.activate_as_host(room.room_id, sub)
  end

  defp maybe_activate_host(room, _role, _sub), do: {:ok, room}

  defp room_session_summary(room, viewer_sub) do
    role_hint = if room.owner_sub == viewer_sub, do: "owner", else: "joinable"

    %{
      room_id: room.room_id,
      visibility: room.visibility,
      owner_sub: room.owner_sub,
      role_hint: role_hint,
      state: room.state,
      active_host_sub: room.active_host_sub
    }
  end

  @doc """
  x0080: Graceful Handover — nominate `target_sub` or pass null to dormant.
  """
  def handover(conn, %{"room_id" => room_id} = params) do
    case resolve_sub(conn, params) do
      {:ok, caller_sub} ->
        target = Map.get(params, "target_sub")

        target_sub =
          cond do
            is_nil(target) -> nil
            target == "" -> nil
            is_binary(target) -> target
            true -> :invalid
          end

        case target_sub do
          :invalid ->
            conn
            |> put_status(:unprocessable_entity)
            |> json(%{error: "invalid_target"})

          target_sub ->
            room_id
            |> Rooms.handover(caller_sub, target_sub)
            |> render_host_session(conn, caller_sub)
        end

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  @doc """
  x0080: Manual Takeover CAS while dormant / lease-expired.
  """
  def takeover(conn, %{"room_id" => room_id} = params) do
    case resolve_sub(conn, params) do
      {:ok, caller_sub} ->
        with {:ok, room} <- Rooms.get(room_id),
             :ok <- RoomAccess.authorize(conn, room.owner_sub, caller_sub, room.visibility),
             {:ok, updated} <- Rooms.takeover(room_id, caller_sub) do
          json(conn, room_session_summary(updated, caller_sub))
        else
          {:error, :conflict} ->
            conn |> put_status(:conflict) |> json(%{error: "takeover_conflict"})

          {:error, :not_found} ->
            conn |> put_status(:not_found) |> json(%{error: "room_not_found"})

          {:error, :invalid_room_id} ->
            conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_room_id"})

          {:error, :storage_error} ->
            storage_unavailable(conn)

          {:error, _} ->
            conn |> put_status(:forbidden) |> json(%{error: "visibility_forbidden"})
        end

      {:error, :anon_disabled} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "anon_tokens_disabled"})
    end
  end

  defp render_host_session({:ok, room}, conn, caller_sub) do
    json(conn, room_session_summary(room, caller_sub))
  end

  defp render_host_session({:error, :not_found}, conn, _caller_sub) do
    conn |> put_status(:not_found) |> json(%{error: "room_not_found"})
  end

  defp render_host_session({:error, :forbidden_host}, conn, _caller_sub) do
    conn |> put_status(:forbidden) |> json(%{error: "handover_forbidden"})
  end

  defp render_host_session({:error, :conflict}, conn, _caller_sub) do
    conn |> put_status(:conflict) |> json(%{error: "handover_conflict"})
  end

  defp render_host_session({:error, :invalid_room_id}, conn, _caller_sub) do
    conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_room_id"})
  end

  defp render_host_session({:error, :invalid_target}, conn, _caller_sub) do
    conn |> put_status(:unprocessable_entity) |> json(%{error: "invalid_target"})
  end

  defp render_host_session({:error, :storage_error}, conn, _caller_sub) do
    storage_unavailable(conn)
  end

  @doc """
  x0025: issue HostAttestation JWT. Authorization must be a **host** Room Token.
  """
  def create_host_key(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
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
          |> json(%{error: "attestation_issue_failed"})
      end
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :pop_required} ->
        pop_unauthorized(conn)

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
        |> json(%{error: "host_role_forbidden"})

      # Lease renew CAS lost (handover / takeover raced) — Token may still verify.
      {:error, :conflict} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "host_lease_conflict"})

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
        Logger.warning("invalid Room Token (attestation path): #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token"})
    end
  end

  @doc """
  x0027: return world-authored scene definition. Authorization must be a **host** Room Token
  whose `sub` is the current ledger `active_host_sub` (x0080).

  Read path checks the ledger only — it does **not** call `renew_host_lease/2`
  (unlike scene write / host_keys / manifest_seal). Heartbeat remains those
  mutating paths (and Token / attestation traffic), so GET-only hosts can still
  hit lease TTL.
  """
  def show_scene(conn, %{"room_id" => room_id}) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         # Ledger gate only; intentionally no renew_host_lease on read.
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         {:ok, scene} <- Rooms.scene(room_id) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :pop_required} ->
        pop_unauthorized(conn)

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
        |> json(%{error: "host_role_forbidden"})

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
        Logger.warning("invalid Room Token (scene path): #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token"})
    end
  end

  @doc """
  x006d: append one mesh SceneEntry to the room scene (host Room Token + active ledger host).

  Body: `%{"blake3" => 64-hex, optional pose/color}`. Assigns `entity_id`,
  bumps revision. Returned scene is the new canonical seal input.
  """
  def create_scene_entry(conn, %{"room_id" => room_id} = params) do
    entry = Map.get(params, "entry") || Map.drop(params, ["room_id"])

    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, scene} <- Rooms.append_scene_entry(room_id, entry) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :pop_required} ->
        pop_unauthorized(conn)

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
        |> json(%{error: "host_role_forbidden"})

      # Lease renew CAS lost (handover / takeover raced) — Token may still verify.
      {:error, :conflict} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "host_lease_conflict"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :invalid_entry} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_entry"})

      {:error, :duplicate_blake3} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "duplicate_blake3"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, :storage_error} ->
        storage_unavailable(conn)

      {:error, reason} ->
        Logger.warning("invalid Room Token (scene entry path): #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token"})
    end
  end

  @doc """
  x007b: remove one mesh SceneEntry by entity_id (host Room Token + active ledger host).

  Requires the entry to carry a 64-hex blake3. Returned scene is the new
  canonical seal input.
  """
  def delete_scene_entry(conn, %{"room_id" => room_id, "entity_id" => entity_id_param}) do
    with {:ok, entity_id} <- parse_entity_id(entity_id_param),
         {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, scene} <- Rooms.delete_scene_entry(room_id, entity_id) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, :invalid_entity_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_entity_id"})

      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :pop_required} ->
        pop_unauthorized(conn)

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
        |> json(%{error: "host_role_forbidden"})

      # Lease renew CAS lost (handover / takeover raced) — Token may still verify.
      {:error, :conflict} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "host_lease_conflict"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :entry_not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "entry_not_found"})

      {:error, :not_mesh_entry} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "not_mesh_entry"})

      {:error, :ambiguous_entity_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "ambiguous_entity_id"})

      {:error, :invalid_room_id} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_room_id"})

      {:error, :storage_error} ->
        storage_unavailable(conn)

      {:error, reason} ->
        Logger.warning("invalid Room Token (scene delete path): #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token"})
    end
  end

  defp parse_entity_id(id) when is_integer(id) and id >= 0, do: {:ok, id}

  defp parse_entity_id(id) when is_binary(id) do
    case Integer.parse(id) do
      {n, ""} when n >= 0 -> {:ok, n}
      _ -> {:error, :invalid_entity_id}
    end
  end

  defp parse_entity_id(_), do: {:error, :invalid_entity_id}

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
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, host_attestation} <- require_host_attestation(params),
         {:ok, host_revision} <- ManifestSeal.parse_revision(Map.get(params, "revision")),
         {:ok, host_blake3} <- ManifestSeal.parse_blake3(Map.get(params, "manifest_blake3")),
         {:ok, revision, host_generation, blake3} <-
           CanonicalManifest.canonical_blake3(room_id, host_attestation),
         :ok <- match_revision(host_revision, revision),
         :ok <- match_blake3(host_blake3, blake3),
         {:ok, _scene_blake3s} <- PlaceHashes.put_for_seal(room_id, revision, blake3) do
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
          |> json(%{error: "manifest_seal_issue_failed"})
      end
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :pop_required} ->
        pop_unauthorized(conn)

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
        |> json(%{error: "host_role_forbidden"})

      # Lease renew CAS lost (handover / takeover raced) — Token may still verify.
      {:error, :conflict} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "host_lease_conflict"})

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
        Logger.warning("invalid Room Token (manifest-seal path): #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_room_token"})
    end
  end

  @doc """
  x006b: issue PlaceRead JWT. Authorization is any Room Token for `room_id`.
  Body requires `world_seal` (ManifestSeal). `extra_blake3` must be absent/empty
  (arbitrary hash carriage disabled until take-out ownership binding).
  Allowlist comes from PlaceHashes captured at seal time — not live scene reload.
  """
  def create_place_read(conn, %{"room_id" => room_id} = params) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, room_claims} <- Token.verify(bearer),
         :ok <- require_room_token(room_claims, room_id),
         {:ok, world_seal} <- require_world_seal(params),
         {:ok, seal_claims} <- verify_world_seal(world_seal),
         :ok <- match_seal_room(seal_claims, room_id),
         {:ok, revision} <- seal_revision(seal_claims),
         {:ok, host_generation} <- seal_host_generation(seal_claims),
         {:ok, manifest_blake3} <- seal_manifest_blake3(seal_claims),
         {:ok, extra} <- PlaceRead.parse_extra_blake3(Map.get(params, "extra_blake3")),
         {:ok, seal_exp} <- PlaceRead.seal_exp_from_claims(seal_claims),
         {:ok, scene_blake3s} <- PlaceHashes.resolve(room_id, revision, manifest_blake3),
         :ok <- touch_place_hashes(room_id, revision, manifest_blake3, seal_exp),
         {:ok, place_read, expires_in} <-
           PlaceRead.generate(
             room_id,
             room_claims["sub"],
             revision,
             host_generation,
             manifest_blake3,
             extra,
             seal_exp
           ) do
      json(conn, %{
        place_read: place_read,
        expires_in: expires_in,
        room_id: room_id,
        revision: revision,
        host_generation: host_generation,
        manifest_blake3: manifest_blake3,
        blake3_count: length(scene_blake3s),
        extra_blake3: [],
        token_type: "Bearer"
      })
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_room_token"})

      {:error, :room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "room_id_mismatch"})

      {:error, :missing_world_seal} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "missing_world_seal"})

      {:error, :seal_room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "seal_room_mismatch"})

      {:error, :revision_mismatch} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "revision_mismatch"})

      {:error, :extra_blake3_disabled} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "extra_blake3_disabled"})

      {:error, :invalid_extra_blake3} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_extra_blake3"})

      {:error, :place_hashes_unavailable} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "place_hashes_unavailable"})

      {:error, :seal_expired} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "seal_expired"})

      {:error, :invalid_seal_exp} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_world_seal"})

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

      {:error, {:invalid_world_seal, reason}} ->
        Logger.warning("place_read invalid world_seal: #{inspect(reason)}")

        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_world_seal"})

      {:error, reason} ->
        Logger.warning("place_read issue failed: #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_place_read_request"})
    end
  end

  @doc """
  x006b: return seal-time scene blake3s for a PlaceRead identity.
  Authorization must be a PlaceRead JWT for `room_id`.
  """
  def show_place_hashes(conn, %{"room_id" => room_id}) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- PlaceRead.verify(bearer),
         :ok <- match_place_read_room(claims, room_id),
         {:ok, revision} <- seal_revision(claims),
         {:ok, manifest_blake3} <- seal_manifest_blake3(claims),
         {:ok, blake3s} <- PlaceHashes.resolve(room_id, revision, manifest_blake3) do
      json(conn, %{
        room_id: room_id,
        revision: revision,
        host_generation: claims["host_generation"],
        manifest_blake3: manifest_blake3,
        blake3s: blake3s,
        extra_blake3: []
      })
    else
      {:error, :missing_bearer} ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_place_read"})

      {:error, :room_mismatch} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "room_id_mismatch"})

      {:error, :place_hashes_unavailable} ->
        conn
        |> put_status(:conflict)
        |> json(%{error: "place_hashes_unavailable"})

      {:error, :revision_mismatch} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "revision_mismatch"})

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "room_not_found"})

      {:error, :storage_error} ->
        storage_unavailable(conn)

      {:error, reason} ->
        Logger.warning("place_hashes failed: #{inspect(reason)}")

        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_place_read"})
    end
  end

  defp require_host_attestation(%{"host_attestation" => token})
       when is_binary(token) and byte_size(token) > 0,
       do: {:ok, token}

  defp require_host_attestation(_), do: {:error, :missing_host_attestation}

  defp require_world_seal(%{"world_seal" => token})
       when is_binary(token) and byte_size(token) > 0,
       do: {:ok, token}

  defp require_world_seal(_), do: {:error, :missing_world_seal}

  defp touch_place_hashes(room_id, revision, manifest_blake3, seal_exp) do
    case PlaceHashes.touch_until(room_id, revision, manifest_blake3, seal_exp) do
      :ok -> :ok
      :miss -> {:error, :place_hashes_unavailable}
      {:error, :seal_expired} = err -> err
    end
  end

  # Tag seal verify failures so Room Token Joken lists are not mistaken for bad seals.
  # Exp failures map to :seal_expired for a stable client contract.
  defp verify_world_seal(token) do
    case ManifestSeal.verify(token) do
      {:ok, claims} ->
        {:ok, claims}

      {:error, reason} ->
        if exp_claim_failure?(reason) do
          {:error, :seal_expired}
        else
          {:error, {:invalid_world_seal, reason}}
        end
    end
  end

  defp exp_claim_failure?(reason) when is_list(reason) do
    Keyword.get(reason, :claim) == "exp" or
      Enum.any?(reason, fn
        {:claim, "exp"} -> true
        _ -> false
      end)
  end

  defp exp_claim_failure?(:expired), do: true
  defp exp_claim_failure?({:error, :expired}), do: true
  defp exp_claim_failure?(_), do: false

  defp require_room_token(%{"room_id" => token_room}, room_id) do
    if token_room == room_id, do: :ok, else: {:error, :room_mismatch}
  end

  defp require_room_token(_, _), do: {:error, :room_mismatch}

  defp match_seal_room(%{"room_id" => seal_room}, room_id) do
    if seal_room == room_id, do: :ok, else: {:error, :seal_room_mismatch}
  end

  defp match_seal_room(_, _), do: {:error, :seal_room_mismatch}

  defp match_place_read_room(%{"room_id" => token_room}, room_id) do
    if token_room == room_id, do: :ok, else: {:error, :room_mismatch}
  end

  defp match_place_read_room(_, _), do: {:error, :room_mismatch}

  defp seal_revision(%{"revision" => rev}) when is_integer(rev) and rev >= 0, do: {:ok, rev}
  defp seal_revision(_), do: {:error, :invalid_revision}

  defp seal_host_generation(%{"host_generation" => gen})
       when is_integer(gen) and gen >= 1,
       do: {:ok, gen}

  defp seal_host_generation(_), do: {:error, :invalid_host_generation}

  defp seal_manifest_blake3(%{"manifest_blake3" => hex}) when is_binary(hex) do
    ManifestSeal.parse_blake3(hex)
  end

  defp seal_manifest_blake3(_), do: {:error, :invalid_manifest_blake3}

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
  # x0081: host routes also require `cnf.spk` PoP (stolen Token alone → 401).
  defp require_host_token(%{"role" => "host", "room_id" => token_room}, room_id) do
    if token_room == room_id do
      :ok
    else
      {:error, :room_mismatch}
    end
  end

  defp require_host_token(_, _), do: {:error, :not_host_token}

  defp require_http_pop(conn, claims) do
    body =
      case conn.assigns do
        %{raw_body: body} when is_binary(body) -> body
        _ -> ""
      end

    case HttpPop.verify(conn, claims, conn.method, conn.request_path, body) do
      :ok -> :ok
      {:error, _} -> {:error, :pop_required}
    end
  end

  defp pop_unauthorized(conn) do
    conn
    |> put_status(:unauthorized)
    |> json(%{error: "pop_required"})
  end

  defp storage_unavailable(conn) do
    conn
    |> put_status(:service_unavailable)
    |> json(%{error: "storage_unavailable"})
  end
end
