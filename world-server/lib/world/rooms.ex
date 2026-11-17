defmodule World.Rooms do
  @moduledoc """
  Room ledger facade (x0028).

  **x00d6 module split:** this module is the stable public facade (create / get / owner /
  editors / eligible hosts / visibility + `defdelegate` for moved APIs). Implementation lives in
  `World.Rooms.Internal` (shared storage-error / overlay-SQL / lease helpers; not public API),
  `World.Rooms.RoomKey` (rotate / grace / due sweep / narrow-rotate CAS),
  `World.Rooms.Catalog` (`list_visible_summaries/3`),
  `World.Rooms.Scene` (canonical + overlay packed CAS, Save, entry append/delete) and
  `World.Rooms.Lease` (host lease / handover / takeover / sweep / host_generation claim).
  Existing `World.Rooms.foo` callers are unchanged; behavior is a pure move.

  Guarantees: create / owner / scene / visibility backed by Ash + SQLite (survives restart).
  x006d / x0082 / **x009d** / **x009e**: `append_scene_entry/4` / `delete_scene_entry/4` are
  **atomic packed CAS** (`expected_canonical` + `expected_overlay` + revision bump).
  **Canonical writer authorization is the caller's responsibility** — HTTP uses
  `ensure_active_host/2` **and** `ensure_scene_writer/2` (owner ∪ `editor_subs`); do not call
  these mutators from an unauthenticated path.
  x0071 / **x0097**: create rejects when owner already holds `max_rooms_per_owner` rooms;
  quota row conditional UPDATE + same-TX insert — concurrent create cannot overshoot.
  x007c: `list_visible_summaries/3` returns catalog rows (no room_key / wire_cap / scene).
  x007e: visibility **narrow** and owner `rotate_room_key/2` bump `room_key_epoch` + new
  `room_key` via **epoch CAS** (lost concurrent rotate retries); previous key retained in
  `RoomKeyGrace` for ≈ Token TTL (memory-only). Periodic tick stamps null
  `room_key_rotated_at` without rotating (deploy baseline; avoids inserted_at stampede).
  x0080: `active_host_sub` / `state` / host lease separate compute host from `owner_sub`;
  Public catalog (`mine_only: false`) is `state = active` with unexpired lease only;
  `mine=1` may include dormant definitions. Handover / takeover / lease sweep are world-authoritative.
  **x009e**: `session_overlay` holds active-host temporary entries (dormant clears);
  `composed_scene/1` merges canonical ∥ overlay; takeover requires owner ∪ `eligible_host_subs`.
  **x00a0**: owner `rotate_room_key/2` is HTTP-reachable; auth relation_changed rotates
  social-visibility rooms owned by either party (CAS).
  **x00a1**: handover nominates `handover_candidate_sub` and goes **dormant**; lease only after
  `accept_handover/2`. Null target → dormant + clear candidate. Owner/active-host may
  **re-nominate** (`handover(other)`) to replace a pending candidate without cancel first.
  Unexpired pending candidate blocks owner Host Token claim and takeover (cancel via
  `handover(nil)` or wait for expiry). `lease_view` nulls expired candidates; sweeper
  durable-clears them.
  **x00a2**: scene/overlay CAS enforces SceneBudget (≤ protocol manifest entries/bytes) and
  composed (canon ∥ overlay) entry cap on **both** canonical and overlay mutate paths.
  **x00b7 / x00b8 / x00b9**: `save_overlay_to_canonical/4` merges overlay into canonical
  (entity_id kept), applies optional `spawns` / `despawns` (host-allocated ids; no remap) and
  live `parents` / `fields` patches, and clears overlay in one packed CAS; empty overlay +
  empty patches is idempotent. HTTP requires active host **and** scene writer; SpawnTicket
  for Save `spawns` is authorized by the HTTP layer before CAS.
  Non-guarantees: full UGC editor; multi-node replication; Postgres (swap path documented);
  entity_id remap on Save collision (reject instead);
  live Reparent/SetField / Spawn CAS (host tip only until Save);
  signed HostDelegation JWT;
  relation notify retry queue; already-issued Room Token JWT revocation (TTL; new epoch AEAD closed);
  grace durability across world restart; Room physical DELETE (and quota decrement);
  user_root / cuboid (empty blake3) despawn;
  catalog Frame reachability; seamless pose / grab inheritance across hosts;
  paginated periodic rotate (due rooms loaded in one tick);
  room-wide scene serialization alone (x0082 is optimistic CAS + host retry);
  world-wide Room total / CCU caps; auto-heal of pre-x0097 overshoot beyond migration backfill.

  x0021: `owner_sub` set only via `create/3` (`{:ok, room_id, visibility, wire_cap}` · x004e).
  Token issuance never creates a room.
  x0027: DemoScene assigned at create and returned by `scene/1`.

  Authorization: owner paths use `ensure_owner/2`; compute-host HTTP uses
  `ensure_active_host/2` / `authorize_host_role` (x0080); canonical scene mutate uses
  `ensure_scene_writer/2` (owner ∪ `editor_subs` · x009d). `require_existing/1` is
  existence-only. Do not infer auth from boolean helpers — those APIs are
  intentionally absent so storage failures cannot collapse to `false` / `nil`.

  Errors: `:invalid_room_id` / `:already_exists` / `:not_found` / `:owner_room_cap_exceeded`
  are client-facing. `:storage_error` is infrastructure (SQLite busy, disk, Repo) → HTTP 5xx.
  Create path: raised `Exqlite.Error` whose message matches SQLITE_BUSY is mapped to
  `:storage_error`; uniqueness / constraint failures are expected as `{:error, _}`
  (Ash / `unique_violation?`) → `:already_exists`, not via raise→rescue.
  """

  require Logger

  import Ecto.Query, only: [from: 2]

  import World.Rooms.Internal,
    only: [
      decode_editor_subs: 1,
      empty_overlay: 0,
      encode_editor_subs: 1,
      encode_eligible_subs: 1,
      map_create_error: 1,
      map_read_error: 2,
      owner_matches: 2,
      set_visibility_only: 2,
      storage_error: 2,
      unique_violation?: 1,
      validate_room_id: 1
    ]

  alias World.DemoScene
  alias World.Repo
  alias World.Rooms.Catalog
  alias World.Rooms.Lease
  alias World.Rooms.OwnerQuota
  alias World.Rooms.Room
  alias World.Rooms.RoomKey
  alias World.Rooms.Scene
  alias World.Visibility

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  @spec create(room_id() | nil, owner_sub(), String.t()) ::
          {:ok, room_id(), String.t(), String.t()}
          | {:error,
             :invalid_room_id
             | :already_exists
             | :missing_owner
             | :invalid_visibility
             | :owner_room_cap_exceeded
             | storage_error()}
  def create(room_id, owner_sub, visibility \\ "public")

  def create(nil, owner_sub, visibility), do: create(random_room_id(), owner_sub, visibility)

  def create(_room_id, owner_sub, _visibility)
      when not is_binary(owner_sub) or owner_sub == "" do
    {:error, :missing_owner}
  end

  def create(room_id, owner_sub, visibility)
      when is_binary(room_id) and is_binary(owner_sub) and is_binary(visibility) do
    with :ok <- validate_room_id(room_id),
         {:ok, vis} <- World.Visibility.parse(visibility),
         {:ok, %Room{room_id: id, visibility: stored, wire_cap: wire_cap}} <-
           create_with_owner_quota(room_id, owner_sub, vis) do
      {:ok, id, stored, wire_cap}
    else
      {:error, :invalid_room_id} = err ->
        err

      {:error, :invalid_visibility} = err ->
        err

      {:error, :owner_room_cap_exceeded} = err ->
        err

      {:error, :already_exists} = err ->
        err

      {:error, :storage_error} = err ->
        err
    end
  end

  def create(_, _, _), do: {:error, :invalid_room_id}

  @doc "Fetch room row once (owner + visibility + scene)."
  @spec get(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  defdelegate get(room_id), to: World.Rooms.Internal, as: :get_room

  defp random_room_id do
    Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
  end

  @doc """
  Validate `room_id` and confirm the room was created (owner already assigned).
  Does not create rooms.
  """
  @spec require_existing(room_id()) ::
          :ok | {:error, :invalid_room_id | :not_found | storage_error()}
  def require_existing(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{}} -> :ok
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :require_existing)
      end
    end
  end

  def require_existing(_), do: {:error, :invalid_room_id}

  @doc "Return the owner sub for an existing room."
  @spec owner(room_id()) ::
          {:ok, owner_sub()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def owner(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{owner_sub: sub}} -> {:ok, sub}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :owner)
      end
    end
  end

  def owner(_), do: {:error, :invalid_room_id}

  @doc "True when `sub` owns the room. Distinguishes not-found / storage from mismatch."
  @spec ensure_owner(room_id(), owner_sub()) ::
          :ok | {:error, :not_found | :forbidden_host | :invalid_room_id | storage_error()}
  def ensure_owner(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{owner_sub: ^sub}} ->
          :ok

        {:ok, %Room{}} ->
          {:error, :forbidden_host}

        {:ok, nil} ->
          {:error, :not_found}

        {:error, error} ->
          map_read_error(error, :ensure_owner)
      end
    end
  end

  def ensure_owner(_, _), do: {:error, :invalid_room_id}

  @doc """
  x009d: `sub` may mutate canonical scene (append/delete) when they are `owner_sub`
  or listed in `editor_subs` (Builder). Distinct from `ensure_active_host/2` (compute).
  """
  @spec ensure_scene_writer(room_id(), owner_sub()) ::
          :ok
          | {:error, :not_found | :forbidden_scene_writer | :invalid_room_id | storage_error()}
  def ensure_scene_writer(room_id, sub)
      when is_binary(room_id) and is_binary(sub) and sub != "" do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{owner_sub: ^sub}} ->
          :ok

        {:ok, %Room{} = room} ->
          authorize_editor_sub(room, sub)

        {:ok, nil} ->
          {:error, :not_found}

        {:error, error} ->
          map_read_error(error, :ensure_scene_writer)
      end
    end
  end

  def ensure_scene_writer(_, _), do: {:error, :invalid_room_id}

  defp authorize_editor_sub(%Room{} = room, sub) do
    if sub in editor_subs(room) do
      :ok
    else
      {:error, :forbidden_scene_writer}
    end
  end

  @doc """
  Decoded `editor_subs` list from the JSON text column (x009d / x00b5).

  Guarantees: same decode as `ensure_scene_writer/2`. Never returns the raw JSON string.
  """
  @spec editor_subs(Room.t() | term()) :: [owner_sub()]
  def editor_subs(%Room{editor_subs: raw}), do: decode_editor_subs(raw)
  def editor_subs(_), do: []

  @doc """
  x009d Phase 2: owner replaces `editor_subs` (Builder list). Empty list = owner-only writers.
  """
  @spec set_editor_subs(room_id(), owner_sub(), [owner_sub()]) ::
          :ok
          | {:error,
             :not_found
             | :forbidden_host
             | :invalid_room_id
             | :invalid_editors
             | storage_error()}
  def set_editor_subs(room_id, owner_sub, editors)
      when is_binary(room_id) and is_binary(owner_sub) and is_list(editors) do
    with :ok <- validate_room_id(room_id),
         :ok <- ensure_owner(room_id, owner_sub),
         {:ok, json} <- encode_editor_subs(editors) do
      sql = """
      UPDATE rooms
      SET editor_subs = ?, updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ? AND owner_sub = ?
      """

      case World.Repo.query(sql, [json, room_id, owner_sub]) do
        {:ok, %{num_rows: 1}} -> :ok
        {:ok, %{num_rows: 0}} -> {:error, :not_found}
        {:error, error} -> storage_error(error, :set_editor_subs)
      end
    end
  end

  def set_editor_subs(_, _, _), do: {:error, :invalid_editors}

  @doc """
  x009e: owner replaces `eligible_host_subs` (takeover allow-list). Empty = owner-only.
  """
  @spec set_eligible_host_subs(room_id(), owner_sub(), [owner_sub()]) ::
          {:ok, [owner_sub()]}
          | {:error,
             :not_found
             | :forbidden_host
             | :invalid_room_id
             | :invalid_eligible
             | storage_error()}
  def set_eligible_host_subs(room_id, owner_sub, subs)
      when is_binary(room_id) and is_binary(owner_sub) and is_list(subs) do
    with :ok <- validate_room_id(room_id),
         :ok <- ensure_owner(room_id, owner_sub),
         {:ok, json} <- encode_eligible_subs(subs) do
      sql = """
      UPDATE rooms
      SET eligible_host_subs = ?, updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ? AND owner_sub = ?
      """

      case World.Repo.query(sql, [json, room_id, owner_sub]) do
        {:ok, %{num_rows: 1}} -> read_eligible_host_subs(room_id)
        {:ok, %{num_rows: 0}} -> {:error, :not_found}
        {:error, error} -> storage_error(error, :set_eligible_host_subs)
      end
    end
  end

  def set_eligible_host_subs(_, _, _), do: {:error, :invalid_eligible}

  defp read_eligible_host_subs(room_id) do
    case get(room_id) do
      {:ok, room} -> {:ok, decode_editor_subs(room.eligible_host_subs)}
      {:error, _} = err -> err
    end
  end

  @doc "Return visibility for an existing room (x0034)."
  @spec visibility(room_id()) ::
          {:ok, String.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def visibility(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{visibility: vis}} when is_binary(vis) -> {:ok, vis}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :visibility)
      end
    end
  end

  def visibility(_), do: {:error, :invalid_room_id}

  @doc """
  Owner updates visibility (x0034).

  x007e: when the new level is **narrower** (more closed) than the previous, rotates
  `room_key` / `room_key_epoch` in the same update and parks the previous key in
  `RoomKeyGrace`.
  """
  @spec set_visibility(room_id(), owner_sub(), String.t()) ::
          {:ok, String.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :invalid_visibility
             | storage_error()}
  def set_visibility(room_id, sub, visibility)
      when is_binary(room_id) and is_binary(sub) and is_binary(visibility) do
    with {:ok, vis} <- Visibility.parse(visibility),
         {:ok, %Room{} = room} <- get(room_id),
         :ok <- owner_matches(room, sub) do
      apply_visibility_change(room, vis)
    end
  end

  def set_visibility(_, _, _), do: {:error, :invalid_room_id}

  defp apply_visibility_change(%Room{} = room, vis) do
    if Visibility.narrower?(vis, room.visibility) do
      RoomKey.narrow_visibility_with_rotate(room, vis)
    else
      set_visibility_only(room, vis)
    end
  end

  # x0097: quota bump + room insert in one TX (assets QuotaLedger pattern).
  # Cheap best-effort hint skips DemoScene/key build on obvious cap reject;
  # conditional UPDATE inside the TX remains the authority. Scene/key material
  # is still built *outside* the write TX so the single-writer lock covers DB
  # only. Mode `:immediate` matches AshSqlite mutate (BEGIN IMMEDIATE).
  defp create_with_owner_quota(room_id, owner_sub, visibility) do
    case max_rooms_per_owner() do
      {:ok, cap} ->
        if owner_cap_hint_exceeded?(owner_sub, cap) do
          {:error, :owner_room_cap_exceeded}
        else
          attrs = new_room_attrs(room_id, owner_sub, visibility)
          commit_create_with_owner_quota(attrs, cap)
        end

      {:error, :invalid_cap} ->
        Logger.error("World.Rooms.create invalid :max_rooms_per_owner (must be pos_integer)")
        {:error, :storage_error}
    end
  rescue
    # Repo stopped / not registered (tests simulate outage before insert).
    e in [RuntimeError, ArgumentError] ->
      storage_error(e, :create)

    # Only SQLITE_BUSY (and message variants). Constraint/unique must not raise here —
    # they return {:error, _} and map to :already_exists via unique_violation?/1.
    e in [Exqlite.Error] ->
      if sqlite_busy_message?(Map.get(e, :message)) do
        storage_error(e, :create)
      else
        reraise e, __STACKTRACE__
      end
  end

  defp sqlite_busy_message?(msg) when is_binary(msg) do
    String.contains?(String.downcase(msg), "busy")
  end

  defp sqlite_busy_message?(_), do: false

  defp commit_create_with_owner_quota(attrs, cap) do
    case Repo.transaction(fn -> create_with_owner_quota!(attrs, cap) end, mode: :immediate) do
      {:ok, %Room{} = room} ->
        {:ok, room}

      {:error, :owner_room_cap_exceeded} ->
        {:error, :owner_room_cap_exceeded}

      {:error, :already_exists} ->
        {:error, :already_exists}

      {:error, :storage_error} ->
        {:error, :storage_error}

      # Ash create failures inside this TX call DataLayer.rollback/2 before
      # returning {:error, _} to the caller — so unique/storage reasons land
      # here as the rollback term (changeset / error class), not via the
      # defensive arm in create_with_owner_quota!/2.
      {:error, error} ->
        map_create_error(error)
    end
  end

  defp create_with_owner_quota!(attrs, cap) do
    owner_sub = attrs.owner_sub
    ensure_owner_quota!(owner_sub)

    case bump_owner_quota(owner_sub, cap) do
      :ok -> :ok
      :cap_exceeded -> Repo.rollback(:owner_room_cap_exceeded)
    end

    case insert_room(attrs) do
      {:ok, %Room{} = room} ->
        room

      # Defensive only: when Ash is already in this TX, create failures abort via
      # Ash.DataLayer.rollback/2 and never return {:error, _} here. Kept so a
      # future non-Ash insert still maps cleanly.
      {:error, error} ->
        if unique_violation?(error) do
          Repo.rollback(:already_exists)
        else
          Logger.error("World.Rooms.create storage failure: #{Exception.message(error)}")
          Repo.rollback(:storage_error)
        end
    end
  end

  defp ensure_owner_quota!(owner_sub) do
    changeset = OwnerQuota.changeset(%OwnerQuota{}, %{owner_sub: owner_sub, room_count: 0})

    case Repo.insert(changeset, on_conflict: :nothing, conflict_target: :owner_sub) do
      {:ok, _} ->
        :ok

      {:error, reason} ->
        Logger.error("World.Rooms.create quota ledger insert failed: #{inspect(reason)}")
        Repo.rollback(:storage_error)
    end
  end

  # Best-effort only: skip expensive attrs when ledger already shows at/over cap.
  # Absent row or under-cap → proceed; TX bump is the real gate.
  defp owner_cap_hint_exceeded?(owner_sub, cap) do
    case Repo.get(OwnerQuota, owner_sub) do
      %OwnerQuota{room_count: count} when is_integer(count) and count >= cap -> true
      _ -> false
    end
  end

  # ensure_owner_quota!/1 just ran in this TX, so n == 0 means at/over cap
  # (missing row would be a bug in ensure — still surfaced as cap_exceeded).
  defp bump_owner_quota(owner_sub, cap) when is_integer(cap) and cap > 0 do
    now = DateTime.utc_now()

    {n, _} =
      from(q in OwnerQuota,
        where: q.owner_sub == ^owner_sub and q.room_count < ^cap
      )
      |> Repo.update_all(inc: [room_count: 1], set: [updated_at: now])

    if n == 1, do: :ok, else: :cap_exceeded
  end

  defp max_rooms_per_owner do
    case Application.get_env(:world, :max_rooms_per_owner, 32) do
      cap when is_integer(cap) and cap > 0 -> {:ok, cap}
      _ -> {:error, :invalid_cap}
    end
  end

  defp new_room_attrs(room_id, owner_sub, visibility) do
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    %{
      room_id: room_id,
      owner_sub: owner_sub,
      scene: DemoScene.build(),
      visibility: visibility,
      wire_cap: generate_wire_cap(),
      room_key: generate_room_key_hex(),
      room_key_epoch: 0,
      room_key_rotated_at: now,
      editor_subs: "[]",
      session_overlay: empty_overlay(),
      eligible_host_subs: "[]"
    }
  end

  defp insert_room(attrs) when is_map(attrs) do
    Room.create(attrs)
  end

  # 16 random bytes → 32 hex chars; matches protocol `[A-Za-z0-9_-]+` / ≤64B.
  defp generate_wire_cap do
    Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
  end

  # ── x00d6: delegates to split modules (public API unchanged) ─────────────────

  # Scene (canonical + overlay CAS · `World.Rooms.Scene`)
  @doc "See `World.Rooms.Scene.pack_revision/2`."
  defdelegate pack_revision(canonical, overlay), to: Scene

  @doc "See `World.Rooms.Scene.unpack_revision/1`."
  defdelegate unpack_revision(packed), to: Scene

  @doc "See `World.Rooms.Scene.scene/1`."
  defdelegate scene(room_id), to: Scene

  @doc "See `World.Rooms.Scene.composed_scene/1`."
  defdelegate composed_scene(room_id), to: Scene

  @doc false
  defdelegate compose_room_scene(room), to: Scene

  @doc "See `World.Rooms.Scene.append_overlay_entry/4`."
  defdelegate append_overlay_entry(room_id, entry, expected_canonical, expected_overlay),
    to: Scene

  @doc "See `World.Rooms.Scene.delete_overlay_entry/4`."
  defdelegate delete_overlay_entry(room_id, entity_id, expected_canonical, expected_overlay),
    to: Scene

  @doc "See `World.Rooms.Scene.save_overlay_to_canonical/3`."
  defdelegate save_overlay_to_canonical(room_id, expected_canonical, expected_overlay),
    to: Scene

  @doc "See `World.Rooms.Scene.save_overlay_to_canonical/4`."
  defdelegate save_overlay_to_canonical(room_id, expected_canonical, expected_overlay, patches),
    to: Scene

  @doc "See `World.Rooms.Scene.append_scene_entry/4`."
  defdelegate append_scene_entry(room_id, entry, expected_canonical, expected_overlay),
    to: Scene

  @doc "See `World.Rooms.Scene.delete_scene_entry/4`."
  defdelegate delete_scene_entry(room_id, entity_id, expected_canonical, expected_overlay),
    to: Scene

  # RoomKey (rotate / grace · `World.Rooms.RoomKey`)
  @doc "See `World.Rooms.RoomKey.rotate_room_key/2`."
  defdelegate rotate_room_key(room_id, sub), to: RoomKey

  @doc "See `World.Rooms.RoomKey.rotate_for_relation_change/2`."
  defdelegate rotate_for_relation_change(sub_a, sub_b), to: RoomKey

  @doc "See `World.Rooms.RoomKey.rotate_due_room_keys/1`."
  defdelegate rotate_due_room_keys(hours), to: RoomKey

  @doc false
  defdelegate generate_room_key_hex(), to: RoomKey

  @doc "See `World.Rooms.RoomKey.room_key_for_token/1`."
  defdelegate room_key_for_token(hex), to: RoomKey

  # Catalog (`World.Rooms.Catalog`)
  @type catalog_summary :: Catalog.catalog_summary()

  @doc "See `World.Rooms.Catalog.list_visible_summaries/3`."
  @spec list_visible_summaries(Plug.Conn.t(), owner_sub(), keyword()) ::
          {:ok, [catalog_summary()]} | {:error, storage_error()}
  def list_visible_summaries(conn, viewer_sub, opts \\ []),
    do: Catalog.list_visible_summaries(conn, viewer_sub, opts)

  # Lease (host lease / handover · `World.Rooms.Lease`)
  @doc "See `World.Rooms.Lease.lease_view_public/1`."
  defdelegate lease_view_public(room), to: Lease

  @doc "See `World.Rooms.Lease.get_session/1`."
  defdelegate get_session(room_id), to: Lease

  @doc "See `World.Rooms.Lease.authorize_host_role/2`."
  defdelegate authorize_host_role(room, sub), to: Lease

  @doc "See `World.Rooms.Lease.activate_as_host/2`."
  defdelegate activate_as_host(room_id, sub), to: Lease

  @doc false
  defdelegate activate_as_host_cas(room_id, sub), to: Lease

  @doc "See `World.Rooms.Lease.renew_host_lease/2`."
  defdelegate renew_host_lease(room_id, sub), to: Lease

  @doc "See `World.Rooms.Lease.ensure_active_host/2`."
  defdelegate ensure_active_host(room_id, sub), to: Lease

  @doc "See `World.Rooms.Lease.handover/3`."
  defdelegate handover(room_id, caller_sub, target_sub), to: Lease

  @doc "See `World.Rooms.Lease.accept_handover/2`."
  defdelegate accept_handover(room_id, caller_sub), to: Lease

  @doc "See `World.Rooms.Lease.takeover/2`."
  defdelegate takeover(room_id, caller_sub), to: Lease

  @doc "See `World.Rooms.Lease.expire_due_host_leases/0`."
  defdelegate expire_due_host_leases(), to: Lease

  @doc "See `World.Rooms.Lease.claim_host_generation/2`."
  defdelegate claim_host_generation(room_id, host_pub), to: Lease

  @doc "See `World.Rooms.Lease.match_claimed_host/3`."
  defdelegate match_claimed_host(room_id, host_pub, host_generation), to: Lease
end
