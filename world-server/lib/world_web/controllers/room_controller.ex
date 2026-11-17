defmodule WorldWeb.RoomController do
  @moduledoc """
  Room HTTP API.

  x00d6 / P2-8: actions return `Plug.Conn` on success and `{:error, reason}` on failure;
  `WorldWeb.FallbackController` renders the HTTP status + JSON `error`.

  Ambiguous domain atoms (`:conflict`, `:forbidden_host`, …) are normalized to unique
  client-facing atoms **here** (the call site knows which API it is serving). SpawnTicket
  release side effects run before an error is returned.
  """

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

  # Atoms that Room Token / host auth paths pass through to FallbackController unchanged.
  # Anything else on those paths is treated as a Room Token verify failure (logged, 401).
  @room_token_passthrough [
    :missing_bearer,
    :pop_required,
    :room_mismatch,
    :room_not_found,
    :invalid_room_id,
    :storage_error
  ]

  # Extra atoms for scene / overlay write + delete paths.
  @scene_write_passthrough [
    :spawn_ticket_required,
    :spawn_ticket_invalid,
    :forbidden_scene_writer,
    :invalid_revision,
    :scene_revision_conflict,
    :scene_entries_exceeded,
    :scene_bytes_exceeded,
    :invalid_scene_revision,
    :invalid_entry,
    :parent_cycle,
    :duplicate_blake3
  ]

  @scene_delete_passthrough [
    :invalid_entity_id,
    :entry_not_found,
    :not_mesh_entry,
    :ambiguous_entity_id
  ]

  @manifest_seal_passthrough [
    :invalid_revision,
    :revision_mismatch,
    :invalid_manifest_blake3,
    :manifest_blake3_mismatch,
    :missing_host_attestation,
    :invalid_host_attestation,
    :attestation_room_mismatch,
    :stale_host_attestation,
    :invalid_scene_revision
  ]

  @place_read_passthrough [
    :missing_bearer,
    :room_mismatch,
    :missing_world_seal,
    :seal_room_mismatch,
    :revision_mismatch,
    :extra_blake3_disabled,
    :invalid_extra_blake3,
    :place_hashes_unavailable,
    :seal_expired,
    :invalid_seal_exp,
    :room_not_found,
    :invalid_room_id,
    :storage_error
  ]

  @token_issue_passthrough [
    :openssl_failed,
    :sign_failed,
    :chmod_failed,
    :invalid_allocation_args,
    :invalid_args,
    :invalid_room_id,
    :invalid_room_key,
    :room_not_found,
    :invalid_revision,
    :visibility_forbidden,
    :lan_required,
    :auth_unavailable,
    :invalid_role,
    :storage_error
  ]

  # x007c: catalog of rooms visible to the caller (no secrets).
  def index(conn, params) do
    with {:ok, viewer_sub} <- resolve_sub(conn, params),
         {:ok, rooms} <-
           Rooms.list_visible_summaries(conn, viewer_sub, mine_only: mine_only_param?(params)) do
      json(conn, %{rooms: rooms})
    end
  end

  defp mine_only_param?(params) do
    case Map.get(params, "mine") do
      v when v in ["1", "true", "TRUE", "yes", "YES"] -> true
      _ -> false
    end
  end

  @doc """
  x00d2: return only `room_key_epoch` for an existing room.

  Authorization is any valid Room Token for `room_id` (guest or host). PoP is
  **not** required — this is a read-only probe so clients can skip Token reissue
  when the epoch is unchanged.
  """
  def show_room_key_epoch(conn, %{"room_id" => room_id}) do
    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_room_token(claims, room_id),
         {:ok, room} <- Rooms.get(room_id) do
      json(conn, %{room_id: room_id, room_key_epoch: room.room_key_epoch})
    else
      {:error, reason} -> normalize_room_token_error(reason, "room_key_epoch")
    end
  end

  # x0080: single-room session summary (no secrets) for Waiting / nomination polls.
  def show(conn, %{"room_id" => room_id} = params) do
    with {:ok, viewer_sub} <- resolve_sub(conn, params) do
      with {:ok, room} <- Rooms.get_session(room_id),
           :ok <- RoomAccess.authorize(conn, room.owner_sub, viewer_sub, room.visibility) do
        json(conn, room_session_summary(room, viewer_sub))
      else
        {:error, reason} -> normalize_show_error(reason)
      end
    end
  end

  defp normalize_show_error(:not_found), do: {:error, :room_not_found}

  defp normalize_show_error(reason) when reason in [:invalid_room_id, :storage_error],
    do: {:error, reason}

  # RoomAccess failures (visibility / lan / auth lookup) are all 403 for show.
  defp normalize_show_error(_reason), do: {:error, :visibility_forbidden}

  def create(conn, params) do
    room_id = Map.get(params, "room_id")
    visibility = Map.get(params, "visibility", "public")

    with {:ok, owner_sub} <- resolve_sub(conn, params),
         {:ok, id, stored_visibility, wire_cap} <- Rooms.create(room_id, owner_sub, visibility) do
      conn
      |> put_status(:created)
      |> json(%{
        room_id: id,
        owner_sub: owner_sub,
        visibility: stored_visibility,
        wire_cap: wire_cap
      })
    end
  end

  # Typical ECDSA/RSA CSR PEM is a few KB; cap rejects disk/openssl DoS bodies.
  @csr_pem_max_bytes 16_384

  def create_token(conn, %{"room_id" => room_id} = params) do
    role = Map.get(params, "role", "guest")

    with {:ok, csr_pem} <- optional_csr_pem(params),
         {:ok, sub} <- resolve_sub(conn, params),
         {:ok, session_pub} <- decode_token_session_pub(params) do
      issue_token(conn, room_id, role, sub, session_pub, csr_pem)
    end
  end

  defp optional_csr_pem(params) do
    case Map.get(params, "csr_pem") do
      pem when is_binary(pem) and pem != "" and byte_size(pem) <= @csr_pem_max_bytes ->
        {:ok, pem}

      pem when is_binary(pem) and byte_size(pem) > @csr_pem_max_bytes ->
        {:error, :csr_pem_too_large}

      _ ->
        {:ok, nil}
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

    with {:ok, sub} <- resolve_sub(conn, params),
         {:ok, stored_visibility} <-
           room_id
           |> Rooms.set_visibility(sub, visibility || "")
           |> normalize_owner_error() do
      json(conn, %{room_id: room_id, visibility: stored_visibility})
    end
  end

  @doc """
  x009e: owner replaces `eligible_host_subs` (takeover allow-list). Body `%{"subs" => [sub…]}`.
  Empty list = owner-only takeover.
  """
  def update_eligible_hosts(conn, %{"room_id" => room_id} = params) do
    subs = Map.get(params, "subs", [])

    with {:ok, sub} <- resolve_sub(conn, params),
         {:ok, stored_subs} when is_list(stored_subs) <-
           room_id
           |> Rooms.set_eligible_host_subs(sub, subs)
           |> normalize_owner_error() do
      json(conn, %{room_id: room_id, eligible_host_subs: stored_subs})
    end
  end

  # Domain `:not_found` → HTTP-boundary `:room_not_found` (shared Fallback keeps generic `:not_found`).
  defp normalize_room_lookup_error({:error, :not_found}), do: {:error, :room_not_found}
  defp normalize_room_lookup_error(other), do: other

  # Owner-only APIs (visibility / eligible hosts / rotate): `:forbidden_host` means "not owner".
  defp normalize_owner_error({:error, :forbidden_host}), do: {:error, :owner_only}
  defp normalize_owner_error(other), do: normalize_room_lookup_error(other)

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

        normalize_token_issue_error(reason)
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

  # Token issue: `:conflict` = host claim CAS lost; `:forbidden_host` = host role denied.
  defp normalize_token_issue_error({:error, :forbidden_host}), do: {:error, :host_role_forbidden}
  defp normalize_token_issue_error({:error, :conflict}), do: {:error, :host_claim_conflict}
  defp normalize_token_issue_error({:error, :not_found}), do: {:error, :room_not_found}
  # Domain ACL atom → HTTP-boundary atom (Fallback table key).
  defp normalize_token_issue_error({:error, :forbidden_visibility}),
    do: {:error, :visibility_forbidden}

  defp normalize_token_issue_error({:error, reason}) when reason in @token_issue_passthrough,
    do: {:error, reason}

  defp normalize_token_issue_error({:error, reason}),
    do: {:error, {:token_issue_failed, reason}}

  defp normalize_token_issue_error(reason), do: {:error, {:token_issue_failed, reason}}

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
    room = Rooms.lease_view_public(room)

    %{
      room_id: room.room_id,
      visibility: room.visibility,
      owner_sub: room.owner_sub,
      role_hint: role_hint,
      state: room.state,
      active_host_sub: room.active_host_sub,
      handover_candidate_sub: room.handover_candidate_sub
    }
  end

  @doc """
  x00a0: Owner Room key rotate (kick / manual revoke entry).
  """
  def rotate_room_key(conn, %{"room_id" => room_id} = params) do
    with {:ok, sub} <- resolve_sub(conn, params),
         {:ok, rotated} <-
           room_id
           |> Rooms.rotate_room_key(sub)
           |> normalize_owner_error(),
         {:ok, epoch} <- rotated_epoch(room_id, rotated) do
      json(conn, %{room_id: room_id, room_key_epoch: epoch})
    end
  end

  defp rotated_epoch(_room_id, epoch) when is_integer(epoch), do: {:ok, epoch}

  defp rotated_epoch(room_id, :noop) do
    with {:ok, room} <- Rooms.get(room_id) |> normalize_room_lookup_error() do
      {:ok, room.room_key_epoch}
    end
  end

  @doc """
  x00a1: Graceful Handover stage A — nominate candidate (dormant) or null → dormant.
  """
  def handover(conn, %{"room_id" => room_id} = params) do
    with {:ok, caller_sub} <- resolve_sub(conn, params),
         {:ok, target_sub} <- parse_handover_target(params),
         {:ok, room} <-
           room_id
           |> Rooms.handover(caller_sub, target_sub)
           |> normalize_handover_error() do
      json(conn, room_session_summary(room, caller_sub))
    end
  end

  defp parse_handover_target(params) do
    case Map.get(params, "target_sub") do
      nil -> {:ok, nil}
      "" -> {:ok, nil}
      target when is_binary(target) -> {:ok, target}
      _ -> {:error, :invalid_target}
    end
  end

  defp normalize_handover_error({:error, :forbidden_host}), do: {:error, :handover_forbidden}
  defp normalize_handover_error({:error, :conflict}), do: {:error, :handover_conflict}
  defp normalize_handover_error({:error, :not_found}), do: {:error, :room_not_found}
  defp normalize_handover_error(other), do: other

  @doc """
  x00a1: Handover stage B — candidate accept CAS → active_host + lease.
  """
  def accept_handover(conn, %{"room_id" => room_id} = params) do
    with {:ok, caller_sub} <- resolve_sub(conn, params),
         {:ok, room} <-
           room_id
           |> Rooms.accept_handover(caller_sub)
           |> normalize_accept_handover_error() do
      json(conn, room_session_summary(room, caller_sub))
    end
  end

  defp normalize_accept_handover_error({:error, :forbidden_host}),
    do: {:error, :handover_accept_forbidden}

  defp normalize_accept_handover_error({:error, :conflict}),
    do: {:error, :handover_accept_conflict}

  defp normalize_accept_handover_error({:error, :not_found}), do: {:error, :room_not_found}
  defp normalize_accept_handover_error(other), do: other

  @doc """
  x0080: Manual Takeover CAS while dormant / lease-expired.
  """
  def takeover(conn, %{"room_id" => room_id} = params) do
    with {:ok, caller_sub} <- resolve_sub(conn, params) do
      with {:ok, room} <- Rooms.get(room_id),
           :ok <- RoomAccess.authorize(conn, room.owner_sub, caller_sub, room.visibility),
           {:ok, updated} <- Rooms.takeover(room_id, caller_sub) do
        json(conn, room_session_summary(updated, caller_sub))
      else
        {:error, reason} -> normalize_takeover_error(reason)
      end
    end
  end

  defp normalize_takeover_error(:conflict), do: {:error, :takeover_conflict}
  defp normalize_takeover_error(:forbidden_host), do: {:error, :takeover_forbidden}
  defp normalize_takeover_error(:not_found), do: {:error, :room_not_found}

  defp normalize_takeover_error(reason)
       when reason in [:invalid_revision, :invalid_room_id, :storage_error],
       do: {:error, reason}

  # RoomAccess failures (visibility / lan / auth lookup).
  defp normalize_takeover_error(_reason), do: {:error, :visibility_forbidden}

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
          {:error, {:attestation_issue_failed, reason}}
      end
    else
      {:error, reason} -> normalize_room_token_error(reason, "attestation", [:invalid_host_pub])
    end
  end

  @doc """
  x0027: return world-authored scene definition. Authorization must be a **host** Room Token
  whose `sub` is the current ledger `active_host_sub` (x0080).

  x0098: response includes `owner_sub` (Room row) so the compute host can authorize
  inventory control from a guest+session whose `sub` matches the owner.

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
         {:ok, room} <- Rooms.get(room_id),
         {:ok, scene} <- Rooms.compose_room_scene(room) do
      # x0098 / x009e: composed scene (canonical ⊕ overlay) + owner_sub for Dual auth.
      json(
        conn,
        scene
        |> Map.put("room_id", room_id)
        |> Map.put("owner_sub", room.owner_sub)
      )
    else
      # Stored scene revision unusable server-side → 500 (not the 422 client-input atom).
      {:error, :invalid_revision} -> {:error, :invalid_revision_server}
      {:error, reason} -> normalize_room_token_error(reason, "scene")
    end
  end

  @doc """
  x006d / x0082 / x009d / x009e / x00b5: append one mesh SceneEntry (host Room Token + active
  ledger host + owner/editor scene writer + assets SpawnTicket).

  Body: `%{"blake3" => 64-hex, "expected_revision" => packed int, "spawn_ticket" => jwt,
  optional pose/color}`. Assigns `entity_id`, atomic packed CAS. Returned scene is
  **composed** (seal input).
  """
  def create_scene_entry(conn, %{"room_id" => room_id} = params) do
    {entry, spawn_ticket} = split_spawn_ticket_entry(params)

    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.ensure_scene_writer(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, packed} <- ManifestSeal.parse_revision(Map.get(params, "expected_revision")),
         {:ok, canon_exp, overlay_exp} <- Rooms.unpack_revision(packed),
         {:ok, ticket_claims} <-
           authorize_spawn_ticket(spawn_ticket, room_id, Map.get(entry, "blake3")),
         {:ok, _raw} <-
           room_id
           |> Rooms.append_scene_entry(entry, canon_exp, overlay_exp)
           |> release_spawn_on_cas_conflict(ticket_claims, entry),
         {:ok, scene} <- Rooms.composed_scene(room_id) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, reason} -> normalize_scene_error(reason, :scene_entry)
    end
  end

  @doc """
  x00b7 / x00b8 / x00b9: merge session_overlay into canonical, fold optional
  `spawns` / `despawns` / `parents` / `fields`, clear overlay (host Room Token +
  active host + owner/editor scene writer). SpawnTicket for each spawn is claimed
  here before packed CAS (x00b5 ownership; live overlay append no longer required).

  Body: `%{"expected_revision" => packed int, "spawns"?, "despawns"?, "parents"?, "fields"?}`.
  Empty overlay + empty patches is idempotent. Returned scene is **composed**.
  """
  def create_scene_save(conn, %{"room_id" => room_id} = params) do
    spawns_raw = Map.get(params, "spawns") || []

    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.ensure_scene_writer(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, packed} <- ManifestSeal.parse_revision(Map.get(params, "expected_revision")),
         {:ok, canon_exp, overlay_exp} <- Rooms.unpack_revision(packed),
         {:ok, claimed} <- authorize_save_spawns(spawns_raw, room_id),
         patches = %{
           "parents" => Map.get(params, "parents") || [],
           "fields" => Map.get(params, "fields") || [],
           "spawns" => strip_spawn_tickets(spawns_raw),
           "despawns" => Map.get(params, "despawns") || []
         },
         {:ok, scene} <-
           room_id
           |> Rooms.save_overlay_to_canonical(canon_exp, overlay_exp, patches)
           |> release_save_spawns_on_cas_conflict(claimed) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, reason} -> normalize_scene_error(reason, :scene_save)
    end
  end

  @doc """
  x007b / x0082 / x009d: remove one mesh SceneEntry by entity_id (host Room Token + active
  ledger host + owner/editor scene writer).

  Body: `%{"expected_revision" => int}`. Requires the entry to carry a 64-hex blake3.
  CAS-bumps revision. Returned scene is the new canonical seal input.
  """
  def delete_scene_entry(conn, %{"room_id" => room_id, "entity_id" => entity_id_param} = params) do
    with {:ok, entity_id} <- parse_entity_id(entity_id_param),
         {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.ensure_scene_writer(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, packed} <- ManifestSeal.parse_revision(Map.get(params, "expected_revision")),
         {:ok, canon_exp, overlay_exp} <- Rooms.unpack_revision(packed),
         {:ok, _raw} <- Rooms.delete_scene_entry(room_id, entity_id, canon_exp, overlay_exp),
         {:ok, scene} <- Rooms.composed_scene(room_id) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, reason} -> normalize_scene_delete_error(reason, "scene delete")
    end
  end

  @doc """
  x009e / x00b5: append one mesh entry to session_overlay (active host + SpawnTicket;
  no scene writer required). Atomic packed CAS on canonical + overlay revisions.
  """
  def create_overlay_entry(conn, %{"room_id" => room_id} = params) do
    {entry, spawn_ticket} = split_spawn_ticket_entry(params)

    with {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, packed} <- ManifestSeal.parse_revision(Map.get(params, "expected_revision")),
         {:ok, canon_exp, overlay_exp} <- Rooms.unpack_revision(packed),
         {:ok, ticket_claims} <-
           authorize_spawn_ticket(spawn_ticket, room_id, Map.get(entry, "blake3")),
         {:ok, scene} <-
           room_id
           |> Rooms.append_overlay_entry(entry, canon_exp, overlay_exp)
           |> release_spawn_on_cas_conflict(ticket_claims, entry) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, reason} -> normalize_scene_error(reason, :overlay_entry)
    end
  end

  @doc """
  x009e: delete one mesh entry from session_overlay by entity_id.
  """
  def delete_overlay_entry(conn, %{"room_id" => room_id, "entity_id" => entity_id_param} = params) do
    with {:ok, entity_id} <- parse_entity_id(entity_id_param),
         {:ok, bearer} <- fetch_bearer(conn),
         {:ok, claims} <- Token.verify(bearer),
         :ok <- require_host_token(claims, room_id),
         :ok <- require_http_pop(conn, claims),
         :ok <- Rooms.ensure_active_host(room_id, claims["sub"]),
         :ok <- Rooms.renew_host_lease(room_id, claims["sub"]),
         {:ok, packed} <- ManifestSeal.parse_revision(Map.get(params, "expected_revision")),
         {:ok, canon_exp, overlay_exp} <- Rooms.unpack_revision(packed),
         {:ok, scene} <-
           Rooms.delete_overlay_entry(room_id, entity_id, canon_exp, overlay_exp) do
      json(conn, Map.put(scene, "room_id", room_id))
    else
      {:error, reason} -> normalize_scene_delete_error(reason, "overlay delete")
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
          {:error, {:manifest_seal_issue_failed, reason}}
      end
    else
      {:error, reason} -> normalize_manifest_seal_error(reason)
    end
  end

  # CLI hash failures: log here, Fallback maps the tagged tuple to the client JSON.
  defp normalize_manifest_seal_error({:cli_missing, _} = reason) do
    Logger.error("manifest-seal-hash missing: #{inspect(reason)}")
    {:error, reason}
  end

  defp normalize_manifest_seal_error({:cli_failed, _, _} = reason) do
    Logger.error("manifest-seal-hash failed: #{inspect(reason)}")
    {:error, reason}
  end

  defp normalize_manifest_seal_error({:unexpected_hash_output, _} = reason) do
    Logger.error("manifest-seal-hash bad output: #{inspect(reason)}")
    {:error, reason}
  end

  # Token.verify / auth failures only — CanonicalManifest/CLI errors must not land here.
  defp normalize_manifest_seal_error(reason),
    do: normalize_room_token_error(reason, "manifest-seal", @manifest_seal_passthrough)

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
      {:error, reason} -> normalize_create_place_read_error(reason)
    end
  end

  defp normalize_create_place_read_error({:invalid_world_seal, reason} = tagged) do
    Logger.warning("place_read invalid world_seal: #{inspect(reason)}")
    {:error, tagged}
  end

  defp normalize_create_place_read_error(:not_found), do: {:error, :room_not_found}

  defp normalize_create_place_read_error(reason) when reason in @place_read_passthrough,
    do: {:error, reason}

  defp normalize_create_place_read_error(reason) do
    Logger.warning("place_read issue failed: #{inspect(reason)}")
    {:error, :invalid_place_read_request}
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
      {:error, reason} -> normalize_place_hashes_error(reason)
    end
  end

  defp normalize_place_hashes_error(:missing_bearer), do: {:error, :missing_place_read}

  defp normalize_place_hashes_error(:not_found), do: {:error, :room_not_found}

  defp normalize_place_hashes_error(reason)
       when reason in [
              :room_mismatch,
              :place_hashes_unavailable,
              :revision_mismatch,
              :storage_error
            ],
       do: {:error, reason}

  defp normalize_place_hashes_error(reason) do
    Logger.warning("place_hashes failed: #{inspect(reason)}")
    {:error, :invalid_place_read}
  end

  # Host / Room Token API failures. Ambiguous domain atoms are made unique here;
  # known client atoms (base + `extra`) pass through; the rest are Token verify failures
  # (logged by FallbackController as `invalid Room Token (<log_ctx> path)`).
  defp normalize_room_token_error(reason, log_ctx, extra \\ [])

  defp normalize_room_token_error(:not_host_token, _log_ctx, _extra),
    do: {:error, :host_role_required}

  defp normalize_room_token_error(:forbidden_host, _log_ctx, _extra),
    do: {:error, :host_role_forbidden}

  # Lease renew CAS lost (handover / takeover raced) — Token may still verify.
  defp normalize_room_token_error(:conflict, _log_ctx, _extra),
    do: {:error, :host_lease_conflict}

  defp normalize_room_token_error(:not_found, _log_ctx, _extra), do: {:error, :room_not_found}

  defp normalize_room_token_error(reason, log_ctx, extra) do
    if reason in @room_token_passthrough or reason in extra do
      {:error, reason}
    else
      {:error, {:invalid_room_token, log_ctx, reason}}
    end
  end

  defp normalize_scene_error(reason, log_ctx),
    do: normalize_room_token_error(reason, log_ctx, @scene_write_passthrough)

  defp normalize_scene_delete_error(reason, log_ctx),
    do:
      normalize_room_token_error(
        reason,
        log_ctx,
        @scene_write_passthrough ++ @scene_delete_passthrough
      )

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

  # x00b5: strip SpawnTicket from scene/overlay append body before Rooms normalize.
  defp split_spawn_ticket_entry(params) do
    raw = Map.get(params, "entry") || Map.drop(params, ["room_id"])

    ticket =
      Map.get(raw, "spawn_ticket") || Map.get(raw, :spawn_ticket) ||
        Map.get(params, "spawn_ticket") || Map.get(params, :spawn_ticket)

    entry =
      raw
      |> Map.drop([
        "expected_revision",
        :expected_revision,
        "spawn_ticket",
        :spawn_ticket
      ])

    {entry, ticket}
  end

  defp authorize_spawn_ticket(ticket, room_id, blake3) do
    World.SpawnTicket.authorize_and_claim(ticket, room_id, blake3 || "")
  end

  # x00b9: claim every Save spawn ticket before packed CAS; release only on CAS conflict.
  defp authorize_save_spawns(spawns, room_id) when is_list(spawns) do
    case Enum.reduce_while(spawns, {:ok, []}, &authorize_one_save_spawn(&1, &2, room_id)) do
      {:ok, acc} -> {:ok, Enum.reverse(acc)}
      {:error, _} = err -> err
    end
  end

  defp authorize_save_spawns(_, _), do: {:error, :invalid_entry}

  defp authorize_one_save_spawn(spawn, {:ok, acc}, room_id) do
    {entry, ticket} = split_spawn_ticket_entry(spawn_as_params(spawn))
    blake3 = Map.get(entry, "blake3") || Map.get(entry, :blake3) || ""

    case authorize_spawn_ticket(ticket, room_id, blake3) do
      {:ok, claims} ->
        {:cont, {:ok, [{claims, blake3} | acc]}}

      {:error, _} = err ->
        release_save_spawn_claims(acc)
        {:halt, err}
    end
  end

  defp spawn_as_params(spawn) when is_map(spawn), do: spawn
  defp spawn_as_params(_), do: %{}

  defp strip_spawn_tickets(spawns) when is_list(spawns) do
    Enum.map(spawns, fn spawn ->
      {entry, _ticket} = split_spawn_ticket_entry(spawn_as_params(spawn))
      entry
    end)
  end

  defp release_spawn_claim(%{"jti" => jti}, blake3) when is_binary(jti) and is_binary(blake3) do
    World.SpawnTicket.release_claim(jti, blake3)
  end

  defp release_spawn_claim(_, _), do: :ok

  defp release_save_spawn_claims(claimed) when is_list(claimed) do
    Enum.each(claimed, fn {claims, blake3} -> release_spawn_claim(claims, blake3) end)
  end

  # Save: release claimed tickets on CAS conflict BEFORE the error reaches FallbackController.
  # Non-CAS failures burn claimed tickets (same fail-closed policy as overlay append).
  defp release_save_spawns_on_cas_conflict({:error, :scene_revision_conflict} = err, claimed) do
    release_save_spawn_claims(claimed)
    err
  end

  defp release_save_spawns_on_cas_conflict(result, _claimed), do: result

  # x00b5: only CAS conflict releases claim (host retry). Other failures burn
  # `(jti,blake3)` — fail-closed; client must re-take_out / re-issue.
  defp release_spawn_on_cas_conflict(
         {:error, :scene_revision_conflict} = err,
         ticket_claims,
         entry
       ) do
    release_spawn_claim(ticket_claims, Map.get(entry, "blake3"))
    err
  end

  defp release_spawn_on_cas_conflict(result, _ticket_claims, _entry), do: result
end
