defmodule World.Rooms.Lease do
  @moduledoc """
  Host lease, handover, takeover and `host_generation` claim (x0080 / x00a1 / x009e; x00d6 split
  from `World.Rooms`).

  Guarantees: lease transitions are SQL CAS on `rooms`; expired leases project as dormant on read
  (`lease_view_public/1`) and are durably cleared by `expire_due_host_leases/0`; overlay clear on
  host change fails closed at the revision stride.

  Non-guarantees: scene mutation semantics, Room key rotation, catalog listing.

  Callers should use the `World.Rooms` facade; this module is the implementation unit.
  """

  require Logger

  import World.Rooms.Internal,
    only: [
      get_room: 1,
      storage_error: 2,
      lease_valid?: 1,
      validate_room_id: 1,
      decode_editor_subs: 1,
      cleared_overlay_sql: 0,
      overlay_clear_headroom_sql: 0,
      miss_after_overlay_clear: 1
    ]

  alias World.Rooms.Room

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  @host_lease_ttl_default_seconds 300

  @doc "Public lease view for HTTP session summaries (expired → dormant projection)."
  @spec lease_view_public(Room.t()) :: Room.t()
  def lease_view_public(%Room{} = room), do: lease_view(room)

  @doc """
  x0080: re-read room with **read-only** lease view (expired active looks dormant).

  Does not write DB — durable expiry is `expire_due_host_leases/0` (sweeper).
  """
  @spec get_session(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def get_session(room_id) when is_binary(room_id) do
    with {:ok, %Room{} = room} <- get_room(room_id) do
      {:ok, lease_view(room)}
    end
  end

  def get_session(_), do: {:error, :invalid_room_id}

  @doc """
  x0080 / x00a1: whether `sub` may receive `role=host` Room Token.

  Allowed when:
  - `state = active`, lease valid, and `sub == active_host_sub`, or
  - session is dormant (or lease-expired), **no unexpired handover candidate**,
    and `sub == owner_sub` (owner resume).
  Pending candidate blocks owner resume — use `accept_handover` or `handover(nil)` cancel.
  """
  @spec authorize_host_role(Room.t(), owner_sub()) :: :ok | {:error, :forbidden_host}
  def authorize_host_role(%Room{} = room, sub) when is_binary(sub) and sub != "" do
    room = lease_view(room)

    cond do
      room.state == "active" and room.active_host_sub == sub ->
        :ok

      room.state == "dormant" and room.owner_sub == sub and is_nil(room.handover_candidate_sub) ->
        :ok

      true ->
        {:error, :forbidden_host}
    end
  end

  def authorize_host_role(_, _), do: {:error, :forbidden_host}

  @doc """
  x0080 / x00a1: mark `sub` as active compute host and renew lease (Host Token / Start path).

  CAS: renew only if already `active_host_sub`, or claim only from dormant / lease-expired
  **without** an unexpired handover candidate. Candidate must `accept_handover/2`;
  owner cancels with `handover(nil)` before resume.
  """
  @spec activate_as_host(room_id(), owner_sub()) ::
          {:ok, Room.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :conflict
             | :invalid_revision
             | storage_error()}
  def activate_as_host(room_id, sub) when is_binary(room_id) and is_binary(sub) and sub != "" do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id),
         room = lease_view(room),
         :ok <- maybe_accept_or_authorize_activate(room, sub),
         :ok <- maybe_cas_handover_accept(room, room_id, sub) do
      cas_activate_host_token(room_id, sub)
    end
  end

  def activate_as_host(_, _), do: {:error, :invalid_room_id}

  defp maybe_accept_or_authorize_activate(%Room{} = room, sub) do
    if pending_handover_candidate?(room) and room.handover_candidate_sub == sub do
      :ok
    else
      authorize_host_role(room, sub)
    end
  end

  # Candidate Host Token path: accept first, then renew under active lease.
  defp maybe_cas_handover_accept(%Room{} = room, room_id, sub) do
    if pending_handover_candidate?(room) do
      case cas_handover_accept(room_id, sub) do
        {:ok, _} -> :ok
        {:error, _} = err -> err
      end
    else
      :ok
    end
  end

  defp pending_handover_candidate?(%Room{} = room) do
    is_binary(room.handover_candidate_sub) and room.handover_candidate_sub != "" and
      match?(%DateTime{}, room.handover_candidate_expires_at) and
      DateTime.compare(room.handover_candidate_expires_at, DateTime.utc_now()) == :gt
  end

  @doc false
  @spec activate_as_host_cas(room_id(), owner_sub()) ::
          {:ok, Room.t()}
          | {:error,
             :invalid_room_id | :not_found | :conflict | :invalid_revision | storage_error()}
  def activate_as_host_cas(room_id, sub)
      when is_binary(room_id) and is_binary(sub) and sub != "" do
    with :ok <- validate_room_id(room_id) do
      cas_activate_host_token(room_id, sub)
    end
  end

  def activate_as_host_cas(_, _), do: {:error, :invalid_room_id}

  @doc """
  x0080: renew lease when caller is the current active host (heartbeat / seal / host_keys).
  """
  @spec renew_host_lease(room_id(), owner_sub()) ::
          :ok
          | {:error,
             :invalid_room_id | :not_found | :forbidden_host | :conflict | storage_error()}
  def renew_host_lease(room_id, sub) when is_binary(room_id) and is_binary(sub) and sub != "" do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{}} <- get_room(room_id) do
      # CAS-only: lost active_host (handover/takeover race) → :conflict, not authorize guess.
      case cas_renew_active_host(room_id, sub) do
        {:ok, _} -> :ok
        other -> other
      end
    end
  end

  def renew_host_lease(_, _), do: {:error, :invalid_room_id}

  @doc """
  x0080: caller must be current `active_host_sub` (lease-valid) for host HTTP paths.
  """
  @spec ensure_active_host(room_id(), owner_sub()) ::
          :ok | {:error, :not_found | :forbidden_host | :invalid_room_id | storage_error()}
  def ensure_active_host(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id) do
      room = lease_view(room)

      if room.state == "active" and room.active_host_sub == sub do
        :ok
      else
        {:error, :forbidden_host}
      end
    end
  end

  def ensure_active_host(_, _), do: {:error, :invalid_room_id}

  @doc """
  x0080 / x00a1: Graceful Handover stage A. Caller must be `active_host_sub` or `owner_sub`.

  `target_sub` binary → record `handover_candidate_sub` and go **dormant** (no lease).
  While a candidate is already pending, another `target_sub` **replaces** it (re-nominate;
  cancel via `nil` is not required first).
  `nil` → dormant and clear candidate. Does not change `owner_sub`.
  Lease is granted only via `accept_handover/2`.
  """
  @spec handover(room_id(), owner_sub(), owner_sub() | nil) ::
          {:ok, Room.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :invalid_target
             | :conflict
             | :invalid_revision
             | storage_error()}
  def handover(room_id, caller_sub, target_sub)

  def handover(room_id, caller_sub, target_sub)
      when is_binary(room_id) and is_binary(caller_sub) and caller_sub != "" and
             (is_nil(target_sub) or (is_binary(target_sub) and target_sub != "")) do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id),
         :ok <- authorize_handover_caller(room, caller_sub) do
      case target_sub do
        nil -> cas_write_dormant(room_id, caller_sub)
        target -> cas_handover_nominate(room_id, caller_sub, target)
      end
    end
  end

  def handover(_, _, _), do: {:error, :invalid_target}

  @doc """
  x00a1: Handover stage B — caller must equal unexpired `handover_candidate_sub`.
  CAS sets active_host + lease, clears candidate and overlay.
  """
  @spec accept_handover(room_id(), owner_sub()) ::
          {:ok, Room.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :conflict
             | :invalid_revision
             | storage_error()}
  def accept_handover(room_id, caller_sub)
      when is_binary(room_id) and is_binary(caller_sub) and caller_sub != "" do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id),
         :ok <- authorize_handover_accept(room, caller_sub) do
      cas_handover_accept(room_id, caller_sub)
    end
  end

  def accept_handover(_, _), do: {:error, :invalid_room_id}

  defp authorize_handover_accept(%Room{} = room, caller_sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    cond do
      room.handover_candidate_sub != caller_sub ->
        {:error, :forbidden_host}

      match?(%DateTime{}, room.handover_candidate_expires_at) and
          DateTime.compare(room.handover_candidate_expires_at, now) != :gt ->
        {:error, :forbidden_host}

      true ->
        :ok
    end
  end

  @doc """
  x0080 / x009e: Manual Takeover CAS — dormant or lease-expired, and caller is
  `owner_sub` or listed in `eligible_host_subs`.
  """
  @spec takeover(room_id(), owner_sub()) ::
          {:ok, Room.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :conflict
             | :invalid_revision
             | storage_error()}
  def takeover(room_id, caller_sub)
      when is_binary(room_id) and is_binary(caller_sub) and caller_sub != "" do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id),
         room = lease_view(room),
         :ok <- authorize_takeover_caller(room, caller_sub),
         :ok <- reject_pending_handover_steal(room) do
      now = DateTime.utc_now() |> DateTime.truncate(:second)
      expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

      # Host identity change: clear overlay (bump revision) so prior host inventory
      # cannot leak into the new host's composed scene — even if sweeper has not run.
      # x00a1: refuse while unexpired candidate remains (SQL also gates).
      sql = """
      UPDATE rooms
      SET active_host_sub = ?,
          state = 'active',
          host_lease_expires_at = ?,
          handover_candidate_sub = NULL,
          handover_candidate_expires_at = NULL,
          session_overlay = #{cleared_overlay_sql()},
          updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ?
        AND (
          state = 'dormant'
          OR host_lease_expires_at IS NULL
          OR host_lease_expires_at <= ?
        )
        AND (
          handover_candidate_sub IS NULL
          OR handover_candidate_expires_at IS NULL
          OR handover_candidate_expires_at <= ?
        )
        AND #{overlay_clear_headroom_sql()}
      """

      case World.Repo.query(sql, [caller_sub, expires, room_id, now, now]) do
        {:ok, %{num_rows: 1}} -> get_room(room_id)
        {:ok, %{num_rows: 0}} -> miss_after_overlay_clear(room_id)
        {:error, error} -> storage_error(error, :takeover)
      end
    end
  end

  def takeover(_, _), do: {:error, :invalid_room_id}

  defp reject_pending_handover_steal(%Room{} = room) do
    if pending_handover_candidate?(room), do: {:error, :conflict}, else: :ok
  end

  defp authorize_takeover_caller(%Room{} = room, sub) do
    cond do
      room.owner_sub == sub -> :ok
      sub in decode_editor_subs(room.eligible_host_subs) -> :ok
      true -> {:error, :forbidden_host}
    end
  end

  @doc """
  x0080 / x009e / x00a1: sweep lease-expired active sessions to dormant, and clear
  expired handover candidates (durable counterpart to `lease_view` projection).

  Returns `{ok_count, err_count}`. `err_count` includes stride-ceiling skips
  (due but cannot clear overlay without GenRev regression) so zombies are visible
  in sweeper metrics.
  """
  @spec expire_due_host_leases() :: {non_neg_integer(), non_neg_integer()}
  def expire_due_host_leases do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    _ = expire_due_handover_candidates(now)

    sql = """
    UPDATE rooms
    SET active_host_sub = NULL,
        state = 'dormant',
        host_lease_expires_at = NULL,
        handover_candidate_sub = NULL,
        handover_candidate_expires_at = NULL,
        session_overlay = #{cleared_overlay_sql()},
        updated_at = CURRENT_TIMESTAMP
    WHERE state = 'active'
      AND (host_lease_expires_at IS NULL OR host_lease_expires_at <= ?)
      AND #{overlay_clear_headroom_sql()}
    """

    case World.Repo.query(sql, [now]) do
      {:ok, %{num_rows: n}} when is_integer(n) ->
        blocked = count_due_overlay_stride_blocked(now)

        if blocked > 0 do
          Logger.warning(
            "host lease sweep: #{blocked} due room(s) blocked by overlay stride ceiling"
          )
        end

        {n, blocked}

      {:ok, _} ->
        {0, count_due_overlay_stride_blocked(now)}

      {:error, error} ->
        _ = storage_error(error, :expire_due_host_leases)
        {0, 1}
    end
  end

  defp expire_due_handover_candidates(now) do
    sql = """
    UPDATE rooms
    SET handover_candidate_sub = NULL,
        handover_candidate_expires_at = NULL,
        updated_at = CURRENT_TIMESTAMP
    WHERE handover_candidate_sub IS NOT NULL
      AND (handover_candidate_expires_at IS NULL OR handover_candidate_expires_at <= ?)
    """

    case World.Repo.query(sql, [now]) do
      {:ok, %{num_rows: n}} when is_integer(n) and n > 0 ->
        Logger.info("handover candidate sweep: expired=#{n}")
        n

      {:ok, _} ->
        0

      {:error, error} ->
        _ = storage_error(error, :expire_due_handover_candidates)
        0
    end
  end

  defp count_due_overlay_stride_blocked(now) do
    sql = """
    SELECT COUNT(*) AS c
    FROM rooms
    WHERE state = 'active'
      AND (host_lease_expires_at IS NULL OR host_lease_expires_at <= ?)
      AND NOT (#{overlay_clear_headroom_sql()})
    """

    case World.Repo.query(sql, [now]) do
      {:ok, %{rows: [[c]]}} when is_integer(c) -> c
      _ -> 0
    end
  end

  defp authorize_handover_caller(%Room{} = room, caller_sub) do
    room = lease_view(room)

    cond do
      room.state == "active" and room.active_host_sub == caller_sub -> :ok
      room.owner_sub == caller_sub -> :ok
      true -> {:error, :forbidden_host}
    end
  end

  # Read-only: expired lease looks dormant; expired handover candidate is cleared
  # without writing (GET / Waiting poll safe). Durable candidate clear is sweeper.
  defp lease_view(%Room{} = room) do
    room =
      if room.state == "active" and not lease_valid?(room.host_lease_expires_at) do
        %{room | state: "dormant", active_host_sub: nil, host_lease_expires_at: nil}
      else
        room
      end

    clear_expired_candidate_projection(room)
  end

  defp clear_expired_candidate_projection(%Room{} = room) do
    now = DateTime.utc_now()

    expired? =
      is_binary(room.handover_candidate_sub) and
        (not match?(%DateTime{}, room.handover_candidate_expires_at) or
           DateTime.compare(room.handover_candidate_expires_at, now) != :gt)

    if expired? do
      %{room | handover_candidate_sub: nil, handover_candidate_expires_at: nil}
    else
      room
    end
  end

  # Host Token path: renew self OR claim from dormant/expired only (never steal live host).
  # Valid-lease self-renew keeps overlay; dormant/expired claim clears+bumps (x009e).
  # x00a1: claim SQL refuses unexpired handover_candidate (owner resume cannot wipe nominate).
  defp cas_activate_host_token(room_id, sub) do
    case cas_renew_active_host_while_lease_valid(room_id, sub) do
      {:ok, _} = ok -> ok
      {:error, :conflict} -> cas_claim_host_clearing_overlay(room_id, sub)
      {:error, _} = err -> err
    end
  end

  # Unlike heartbeat `cas_renew_active_host/2`, refuse expired leases so activate
  # falls through to clear-overlay claim (session effectively ended).
  defp cas_renew_active_host_while_lease_valid(room_id, sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

    sql = """
    UPDATE rooms
    SET host_lease_expires_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND state = 'active'
      AND active_host_sub = ?
      AND host_lease_expires_at IS NOT NULL
      AND host_lease_expires_at > ?
    """

    case World.Repo.query(sql, [expires, room_id, sub, now]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> {:error, :conflict}
      {:error, error} -> storage_error(error, :activate_as_host)
    end
  end

  defp cas_claim_host_clearing_overlay(room_id, sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

    sql = """
    UPDATE rooms
    SET active_host_sub = ?,
        state = 'active',
        host_lease_expires_at = ?,
        handover_candidate_sub = NULL,
        handover_candidate_expires_at = NULL,
        session_overlay = #{cleared_overlay_sql()},
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND (
        state = 'dormant'
        OR host_lease_expires_at IS NULL
        OR host_lease_expires_at <= ?
      )
      AND (
        handover_candidate_sub IS NULL
        OR handover_candidate_expires_at IS NULL
        OR handover_candidate_expires_at <= ?
      )
      AND #{overlay_clear_headroom_sql()}
    """

    case World.Repo.query(sql, [sub, expires, room_id, now, now]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> miss_after_overlay_clear(room_id)
      {:error, error} -> storage_error(error, :activate_as_host)
    end
  end

  defp cas_renew_active_host(room_id, sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

    sql = """
    UPDATE rooms
    SET host_lease_expires_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND state = 'active'
      AND active_host_sub = ?
    """

    case World.Repo.query(sql, [expires, room_id, sub]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> {:error, :conflict}
      {:error, error} -> storage_error(error, :renew_host_lease)
    end
  end

  # x00a1: nominate stores candidate and goes dormant — no lease until accept.
  defp cas_handover_nominate(room_id, caller_sub, target_sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    cand_expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

    sql = """
    UPDATE rooms
    SET active_host_sub = NULL,
        state = 'dormant',
        host_lease_expires_at = NULL,
        handover_candidate_sub = ?,
        handover_candidate_expires_at = ?,
        session_overlay = #{cleared_overlay_sql()},
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND (
        (state = 'active' AND active_host_sub = ?)
        OR owner_sub = ?
      )
      AND #{overlay_clear_headroom_sql()}
    """

    case World.Repo.query(sql, [target_sub, cand_expires, room_id, caller_sub, caller_sub]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> miss_after_overlay_clear(room_id)
      {:error, error} -> storage_error(error, :handover)
    end
  end

  defp cas_handover_accept(room_id, caller_sub) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    expires = DateTime.add(now, host_lease_ttl_seconds(), :second)

    sql = """
    UPDATE rooms
    SET active_host_sub = ?,
        state = 'active',
        host_lease_expires_at = ?,
        handover_candidate_sub = NULL,
        handover_candidate_expires_at = NULL,
        session_overlay = #{cleared_overlay_sql()},
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND handover_candidate_sub = ?
      AND handover_candidate_expires_at IS NOT NULL
      AND handover_candidate_expires_at > ?
      AND (
        state = 'dormant'
        OR host_lease_expires_at IS NULL
        OR host_lease_expires_at <= ?
      )
      AND #{overlay_clear_headroom_sql()}
    """

    case World.Repo.query(sql, [caller_sub, expires, room_id, caller_sub, now, now]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> miss_after_overlay_clear(room_id)
      {:error, error} -> storage_error(error, :accept_handover)
    end
  end

  defp cas_write_dormant(room_id, caller_sub) do
    sql = """
    UPDATE rooms
    SET active_host_sub = NULL,
        state = 'dormant',
        host_lease_expires_at = NULL,
        handover_candidate_sub = NULL,
        handover_candidate_expires_at = NULL,
        session_overlay = #{cleared_overlay_sql()},
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND (
        (state = 'active' AND active_host_sub = ?)
        OR owner_sub = ?
      )
      AND #{overlay_clear_headroom_sql()}
    """

    case World.Repo.query(sql, [room_id, caller_sub, caller_sub]) do
      {:ok, %{num_rows: 1}} -> get_room(room_id)
      {:ok, %{num_rows: 0}} -> miss_after_overlay_clear(room_id)
      {:error, error} -> storage_error(error, :handover)
    end
  end

  defp host_lease_ttl_seconds do
    case Application.get_env(:world, :host_lease_ttl_seconds, @host_lease_ttl_default_seconds) do
      n when is_integer(n) and n > 0 -> n
      _ -> @host_lease_ttl_default_seconds
    end
  end

  # Concurrent host_keys (owner dual-start overlap) must not mint the same generation
  # for two different pubs — optimistic CAS + retry (x003a).
  @claim_host_generation_attempts 8

  @doc """
  x003a: assign / reuse `host_generation` for an attested host_pub.

  Same pub → keep generation (CAS confirm). Different pub (or first claim) → bump via CAS.
  Returns generation >= 1. Safe under concurrent `host_keys` for the same room.
  """
  @spec claim_host_generation(room_id(), binary()) ::
          {:ok, pos_integer()}
          | {:error, :invalid_room_id | :not_found | :invalid_host_pub | storage_error()}
  def claim_host_generation(room_id, host_pub)
      when is_binary(room_id) and is_binary(host_pub) and byte_size(host_pub) == 32 do
    with :ok <- validate_room_id(room_id) do
      encoded = Base.url_encode64(host_pub, padding: false)
      claim_host_generation_cas(room_id, encoded, @claim_host_generation_attempts)
    end
  end

  def claim_host_generation(_, host_pub) when is_binary(host_pub), do: {:error, :invalid_host_pub}
  def claim_host_generation(_, _), do: {:error, :invalid_room_id}

  @doc """
  x003a: seal path — attestation `(host_pub, host_generation)` must match Room ledger.
  Rejects TTL-valid but superseded attestations.
  """
  @spec match_claimed_host(room_id(), binary(), pos_integer()) ::
          :ok
          | {:error,
             :invalid_room_id
             | :not_found
             | :invalid_host_pub
             | :stale_host_attestation
             | storage_error()}
  def match_claimed_host(room_id, host_pub, host_generation)
      when is_binary(room_id) and is_binary(host_pub) and byte_size(host_pub) == 32 and
             is_integer(host_generation) and host_generation >= 1 do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- get_room(room_id) do
      encoded = Base.url_encode64(host_pub, padding: false)

      if room.attested_host_pub == encoded and room.host_generation == host_generation do
        :ok
      else
        {:error, :stale_host_attestation}
      end
    end
  end

  def match_claimed_host(_, host_pub, _) when is_binary(host_pub), do: {:error, :invalid_host_pub}
  def match_claimed_host(_, _, _), do: {:error, :invalid_room_id}

  defp claim_host_generation_cas(_room_id, _encoded, 0) do
    Logger.error("World.Rooms.claim_host_generation CAS exhausted")
    {:error, :storage_error}
  end

  defp claim_host_generation_cas(room_id, encoded, attempts_left) do
    case get_room(room_id) do
      {:ok, %Room{} = room} ->
        case try_claim_once(room_id, room, encoded) do
          {:ok, gen} ->
            {:ok, gen}

          :conflict ->
            claim_host_generation_cas(room_id, encoded, attempts_left - 1)

          {:error, error} ->
            storage_error(error, :claim_host_generation)
        end

      other ->
        other
    end
  end

  defp try_claim_once(room_id, %Room{} = room, encoded) do
    gen = room.host_generation || 0

    if same_claimed_host?(room, encoded, gen) do
      map_cas_result(cas_confirm_same_host(room_id, gen, encoded), gen)
    else
      next = max(gen, 0) + 1
      map_cas_result(cas_bump_host(room_id, gen, room.attested_host_pub, next, encoded), next)
    end
  end

  defp same_claimed_host?(%Room{attested_host_pub: pub}, encoded, gen)
       when pub == encoded and is_integer(gen) and gen >= 1,
       do: true

  defp same_claimed_host?(_, _, _), do: false

  defp map_cas_result(:ok, gen), do: {:ok, gen}
  defp map_cas_result(:conflict, _gen), do: :conflict
  defp map_cas_result({:error, error}, _gen), do: {:error, error}

  # Touch-update only succeeds while (generation, pub) still match — concurrent bump loses.
  defp cas_confirm_same_host(room_id, expected_gen, encoded) do
    sql = """
    UPDATE rooms
    SET updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND host_generation = ?
      AND attested_host_pub = ?
    """

    case World.Repo.query(sql, [room_id, expected_gen, encoded]) do
      {:ok, %{num_rows: 1}} -> :ok
      {:ok, %{num_rows: 0}} -> :conflict
      {:error, error} -> {:error, error}
    end
  end

  defp cas_bump_host(room_id, expected_gen, expected_pub, next_gen, new_pub) do
    {sql, params} =
      if is_nil(expected_pub) do
        {"""
         UPDATE rooms
         SET host_generation = ?, attested_host_pub = ?, updated_at = CURRENT_TIMESTAMP
         WHERE room_id = ?
           AND host_generation = ?
           AND attested_host_pub IS NULL
         """, [next_gen, new_pub, room_id, expected_gen]}
      else
        {"""
         UPDATE rooms
         SET host_generation = ?, attested_host_pub = ?, updated_at = CURRENT_TIMESTAMP
         WHERE room_id = ?
           AND host_generation = ?
           AND attested_host_pub = ?
         """, [next_gen, new_pub, room_id, expected_gen, expected_pub]}
      end

    case World.Repo.query(sql, params) do
      {:ok, %{num_rows: 1}} -> :ok
      {:ok, %{num_rows: 0}} -> :conflict
      {:error, error} -> {:error, error}
    end
  end
end
