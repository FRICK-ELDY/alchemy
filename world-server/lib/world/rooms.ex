defmodule World.Rooms do
  @moduledoc """
  Room ledger facade (x0028).

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
  Non-guarantees: full UGC editor; multi-node replication; Postgres (swap path documented);
  overlay→canonical Save/merge; signed HostDelegation JWT;
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
  """

  require Logger

  import Ecto.Query, only: [from: 2]

  alias World.DemoScene
  alias World.Repo
  alias World.Rooms.OwnerQuota
  alias World.Rooms.Room
  alias World.Rooms.RoomKeyGrace
  alias World.Visibility

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  # Same budget as host_generation CAS (x003a) — concurrent rotate / narrow must not
  # overwrite an intermediate epoch without parking its key in grace.
  @rotate_cas_attempts 8

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
      {:error, :invalid_room_id} = err -> err
      {:error, :invalid_visibility} = err -> err
      {:error, :owner_room_cap_exceeded} = err -> err
      {:error, :already_exists} = err -> err
      {:error, :storage_error} = err -> err
      {:error, error} -> map_create_error(error)
    end
  end

  def create(_, _, _), do: {:error, :invalid_room_id}

  @doc "Fetch room row once (owner + visibility + scene)."
  @spec get(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def get(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{} = room} -> {:ok, room}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :get)
      end
    end
  end

  def get(_), do: {:error, :invalid_room_id}

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
    if sub in decode_editor_subs(room.editor_subs) do
      :ok
    else
      {:error, :forbidden_scene_writer}
    end
  end

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

  @revision_stride 1_000_000

  @doc """
  x009e: pack canonical + overlay revisions for host-facing scene JSON.

  Each component must be `< 1_000_000` (stride). Clear/bump never decreases packed
  revision so client GenRev watermarks stay monotonic within a `host_generation`.
  """
  @spec pack_revision(non_neg_integer(), non_neg_integer()) ::
          {:ok, non_neg_integer()} | {:error, :invalid_revision}
  def pack_revision(canonical, overlay)
      when is_integer(canonical) and canonical >= 0 and canonical < @revision_stride and
             is_integer(overlay) and overlay >= 0 and overlay < @revision_stride do
    {:ok, canonical * @revision_stride + overlay}
  end

  def pack_revision(_, _), do: {:error, :invalid_revision}

  @doc "x009e: unpack host `expected_revision` into `{canonical, overlay}`."
  @spec unpack_revision(non_neg_integer()) ::
          {:ok, non_neg_integer(), non_neg_integer()} | {:error, :invalid_revision}
  def unpack_revision(packed) when is_integer(packed) and packed >= 0 do
    canonical = div(packed, @revision_stride)
    overlay = rem(packed, @revision_stride)

    if canonical < @revision_stride do
      {:ok, canonical, overlay}
    else
      {:error, :invalid_revision}
    end
  end

  def unpack_revision(_), do: {:error, :invalid_revision}

  @doc "Return the scene definition for an existing room (x0027)."
  @spec scene(room_id()) ::
          {:ok, map()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def scene(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{scene: scene}} when is_map(scene) -> {:ok, scene}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :scene)
      end
    end
  end

  def scene(_), do: {:error, :invalid_room_id}

  @doc """
  x009e: canonical scene ⊕ session_overlay for GET / ManifestSeal.

  Returns a DemoScene-shaped map with packed `revision` and merged `entries`.
  """
  @spec composed_scene(room_id()) ::
          {:ok, map()}
          | {:error, :invalid_room_id | :not_found | :invalid_revision | storage_error()}
  def composed_scene(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id) do
      compose_room_scene(room)
    end
  end

  def composed_scene(_), do: {:error, :invalid_room_id}

  @doc false
  @spec compose_room_scene(Room.t()) :: {:ok, map()} | {:error, :invalid_revision}
  def compose_room_scene(%Room{} = room) do
    scene = if is_map(room.scene), do: room.scene, else: %{}

    with {:ok, overlay} <- require_overlay(room.session_overlay),
         {:ok, canon_rev} <- scene_revision(scene),
         {:ok, overlay_rev} <- scene_revision(overlay),
         {:ok, packed} <- pack_revision(canon_rev, overlay_rev) do
      {:ok,
       %{
         "revision" => packed,
         "locomotion" => Map.get(scene, "locomotion") || Map.get(scene, :locomotion),
         "entries" => scene_entries(scene) ++ scene_entries(overlay)
       }}
    else
      {:error, :invalid_scene_revision} -> {:error, :invalid_revision}
      {:error, :invalid_revision} = err -> err
    end
  end

  @doc """
  x009e: append one mesh entry to `session_overlay` (active host temporary).

  Allocates `entity_id` against composed entries (no collision with canonical).
  Atomic packed CAS: SQL requires both canonical and overlay revisions.
  """
  @spec append_overlay_entry(room_id(), map(), non_neg_integer(), non_neg_integer()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :invalid_entry
             | :duplicate_blake3
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | :scene_entries_exceeded
             | :scene_bytes_exceeded
             | storage_error()}
  def append_overlay_entry(room_id, entry, expected_canonical_revision, expected_overlay_revision)
      when is_binary(room_id) and is_map(entry) and is_integer(expected_canonical_revision) and
             expected_canonical_revision >= 0 and is_integer(expected_overlay_revision) and
             expected_overlay_revision >= 0 do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id),
         {:ok, overlay} <- require_overlay(room.session_overlay) do
      composed_entries = scene_entries(room.scene) ++ scene_entries(overlay)

      with {:ok, current_overlay} <- scene_revision(overlay),
           {:ok, current_canon} <- scene_revision(room.scene),
           :ok <- match_expected_revision(current_overlay, expected_overlay_revision),
           :ok <- match_expected_revision(current_canon, expected_canonical_revision),
           :ok <- ensure_revision_headroom(expected_overlay_revision),
           {:ok, normalized} <- normalize_mesh_entry(entry),
           :ok <- reject_duplicate_blake3(composed_entries, normalized["blake3"]),
           {:ok, next_overlay} <-
             put_overlay_entry(overlay, composed_entries, normalized, expected_overlay_revision),
           :ok <-
             cas_set_overlay(
               room_id,
               expected_canonical_revision,
               expected_overlay_revision,
               next_overlay
             ),
           {:ok, fresh} <- fetch_room(room_id) do
        compose_room_scene(fresh)
      end
    end
  end

  def append_overlay_entry(_, _, _, _), do: {:error, :invalid_room_id}

  @doc """
  x009e: delete one mesh entry from `session_overlay` by entity_id.
  """
  @spec delete_overlay_entry(room_id(), non_neg_integer(), non_neg_integer(), non_neg_integer()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :entry_not_found
             | :not_mesh_entry
             | :ambiguous_entity_id
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | storage_error()}
  def delete_overlay_entry(
        room_id,
        entity_id,
        expected_canonical_revision,
        expected_overlay_revision
      )
      when is_binary(room_id) and is_integer(entity_id) and entity_id >= 0 and
             is_integer(expected_canonical_revision) and expected_canonical_revision >= 0 and
             is_integer(expected_overlay_revision) and expected_overlay_revision >= 0 do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id),
         {:ok, overlay} <- require_overlay(room.session_overlay) do
      with {:ok, current_overlay} <- scene_revision(overlay),
           {:ok, current_canon} <- scene_revision(room.scene),
           :ok <- match_expected_revision(current_overlay, expected_overlay_revision),
           :ok <- match_expected_revision(current_canon, expected_canonical_revision),
           :ok <- ensure_revision_headroom(expected_overlay_revision),
           {:ok, next_overlay} <-
             build_deleted_scene(overlay, entity_id, expected_overlay_revision),
           :ok <-
             cas_set_overlay(
               room_id,
               expected_canonical_revision,
               expected_overlay_revision,
               next_overlay
             ),
           {:ok, fresh} <- fetch_room(room_id) do
        compose_room_scene(fresh)
      end
    end
  end

  def delete_overlay_entry(_, _, _, _), do: {:error, :invalid_room_id}

  @doc """
  Append one mesh SceneEntry (x006d / x0082 / x009e).

  **Auth:** not checked here (x009d). Callers must have already authorized a canonical
  scene writer (HTTP: `ensure_active_host` + `ensure_scene_writer`).

  Assigns the next free content `entity_id` (max non-user_root id + 1, floor 100;
  if the candidate lands in the Presence user_root band, jumps to
  `World.Protocol.user_root_entity_after_band/0` — x009b).
  Atomic packed CAS on canonical + overlay revisions. Returns the updated **canonical**
  scene map (HTTP composes separately).
  """
  @spec append_scene_entry(room_id(), map(), non_neg_integer(), non_neg_integer()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :invalid_entry
             | :duplicate_blake3
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | :scene_entries_exceeded
             | :scene_bytes_exceeded
             | storage_error()}
  def append_scene_entry(room_id, entry, expected_canonical_revision, expected_overlay_revision)
      when is_binary(room_id) and is_map(entry) and is_integer(expected_canonical_revision) and
             expected_canonical_revision >= 0 and is_integer(expected_overlay_revision) and
             expected_overlay_revision >= 0 do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id),
         {:ok, overlay} <- require_overlay(room.session_overlay),
         {:ok, current_canon} <- scene_revision(room.scene),
         {:ok, current_overlay} <- scene_revision(overlay),
         :ok <- match_expected_revision(current_canon, expected_canonical_revision),
         :ok <- match_expected_revision(current_overlay, expected_overlay_revision),
         :ok <- ensure_revision_headroom(expected_canonical_revision),
         {:ok, normalized} <- normalize_mesh_entry(entry),
         {:ok, next_scene} <-
           build_appended_scene(room.scene, normalized, expected_canonical_revision),
         :ok <-
           cas_set_scene(
             room_id,
             expected_canonical_revision,
             expected_overlay_revision,
             next_scene
           ) do
      {:ok, next_scene}
    end
  end

  def append_scene_entry(_, _, _, _), do: {:error, :invalid_room_id}

  @doc """
  Remove one mesh SceneEntry by `entity_id` (x007b / x0082 / x009e).

  **Auth:** not checked here (x009d). Callers must have already authorized a canonical
  scene writer (HTTP: `ensure_active_host` + `ensure_scene_writer`).

  Requires the entry to carry a 64-hex `blake3` (same selection as put-back).
  Rejects missing entity, empty/invalid blake3, and non-list scenes.
  Atomic packed CAS on canonical + overlay revisions. Returns the updated **canonical**
  scene map.
  """
  @spec delete_scene_entry(room_id(), non_neg_integer(), non_neg_integer(), non_neg_integer()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :entry_not_found
             | :not_mesh_entry
             | :ambiguous_entity_id
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | storage_error()}
  def delete_scene_entry(
        room_id,
        entity_id,
        expected_canonical_revision,
        expected_overlay_revision
      )
      when is_binary(room_id) and is_integer(entity_id) and entity_id >= 0 and
             is_integer(expected_canonical_revision) and expected_canonical_revision >= 0 and
             is_integer(expected_overlay_revision) and expected_overlay_revision >= 0 do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id),
         {:ok, overlay} <- require_overlay(room.session_overlay),
         {:ok, current_canon} <- scene_revision(room.scene),
         {:ok, current_overlay} <- scene_revision(overlay),
         :ok <- match_expected_revision(current_canon, expected_canonical_revision),
         :ok <- match_expected_revision(current_overlay, expected_overlay_revision),
         :ok <- ensure_revision_headroom(expected_canonical_revision),
         {:ok, next_scene} <-
           build_deleted_scene(room.scene, entity_id, expected_canonical_revision),
         :ok <-
           cas_set_scene(
             room_id,
             expected_canonical_revision,
             expected_overlay_revision,
             next_scene
           ) do
      {:ok, next_scene}
    end
  end

  def delete_scene_entry(_, _, _, _), do: {:error, :invalid_room_id}

  defp scene_revision(scene) when is_map(scene) do
    case Map.get(scene, "revision") || Map.get(scene, :revision) do
      rev when is_integer(rev) and rev >= 0 -> {:ok, rev}
      _ -> {:error, :invalid_scene_revision}
    end
  end

  defp scene_revision(_), do: {:error, :invalid_scene_revision}

  defp match_expected_revision(current, expected) when current == expected, do: :ok
  defp match_expected_revision(_, _), do: {:error, :scene_revision_conflict}

  defp ensure_revision_headroom(expected)
       when is_integer(expected) and expected >= 0 and expected + 1 < @revision_stride,
       do: :ok

  defp ensure_revision_headroom(_), do: {:error, :invalid_revision}

  # Optimistic packed CAS: both canonical and overlay revisions must match (x009e).
  # x00a2: SceneBudget on next canonical body + composed (canon∥overlay) entry cap.
  defp cas_set_scene(room_id, expected_canon, expected_overlay, next_scene)
       when is_binary(room_id) and is_integer(expected_canon) and is_integer(expected_overlay) and
              is_map(next_scene) do
    with {:ok, %Room{} = room} <- fetch_room(room_id),
         {:ok, overlay} <- require_overlay(room.session_overlay),
         :ok <- World.SceneBudget.ensure_composed(next_scene, overlay),
         {:ok, encoded} <- World.SceneBudget.encode_checked(next_scene) do
      sql = """
      UPDATE rooms
      SET scene = ?,
          updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ?
        AND CAST(json_extract(scene, '$.revision') AS INTEGER) = ?
        AND CAST(json_extract(session_overlay, '$.revision') AS INTEGER) = ?
      """

      # Distinguish vanished room from lost CAS race.
      # If the row still exists we return conflict (409). That includes rare cases where
      # `json_extract` no longer matches after a concurrent corrupt write that slipped past
      # the early `scene_revision/1` check — host may 409-retry once, then hit
      # `:invalid_scene_revision` (500) on the next fetch. Acceptable; not treated as 404.
      cas_packed_update_result(
        World.Repo.query(sql, [encoded, room_id, expected_canon, expected_overlay]),
        room_id,
        :cas_set_scene
      )
    end
  end

  defp cas_set_overlay(room_id, expected_canon, expected_overlay, next_overlay)
       when is_binary(room_id) and is_integer(expected_canon) and is_integer(expected_overlay) and
              is_map(next_overlay) do
    with {:ok, %Room{} = room} <- fetch_room(room_id),
         :ok <- World.SceneBudget.ensure_composed(room.scene, next_overlay),
         {:ok, encoded} <- World.SceneBudget.encode_checked(next_overlay) do
      sql = """
      UPDATE rooms
      SET session_overlay = ?,
          updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ?
        AND CAST(json_extract(scene, '$.revision') AS INTEGER) = ?
        AND CAST(json_extract(session_overlay, '$.revision') AS INTEGER) = ?
      """

      cas_packed_update_result(
        World.Repo.query(sql, [encoded, room_id, expected_canon, expected_overlay]),
        room_id,
        :cas_set_overlay
      )
    end
  end

  defp cas_packed_update_result({:ok, %{num_rows: 1}}, _room_id, _context), do: :ok

  defp cas_packed_update_result({:ok, %{num_rows: 0}}, room_id, _context) do
    case fetch_room(room_id) do
      {:ok, _} -> {:error, :scene_revision_conflict}
      {:error, _} = err -> err
    end
  end

  defp cas_packed_update_result({:error, error}, _room_id, context),
    do: storage_error(error, context)

  defp empty_overlay, do: %{"revision" => 0, "entries" => []}

  # Bump overlay revision and drop entries so packed GenRev never regresses (x009e).
  # Call sites MUST also AND `overlay_clear_headroom_sql/0` so stride overflow fail-closes
  # (UPDATE num_rows=0) instead of writing revision >= 1_000_000.
  defp cleared_overlay_sql do
    """
    json_object(
      'revision', COALESCE(CAST(json_extract(session_overlay, '$.revision') AS INTEGER), 0) + 1,
      'entries', json_array()
    )
    """
  end

  defp overlay_clear_headroom_sql do
    "COALESCE(CAST(json_extract(session_overlay, '$.revision') AS INTEGER), 0) + 1 < #{@revision_stride}"
  end

  # Shared by compose + mutation: never coerce corrupt revision / undecodable blob to 0.
  # `nil` only (fresh row) → empty overlay. Bad JSON / non-integer revision → `:invalid_revision`.
  defp require_overlay(nil), do: {:ok, empty_overlay()}

  defp require_overlay(overlay) when is_map(overlay) do
    case scene_revision(overlay) do
      {:ok, rev} ->
        {:ok, %{"revision" => rev, "entries" => scene_entries(overlay)}}

      {:error, :invalid_scene_revision} ->
        {:error, :invalid_revision}
    end
  end

  defp require_overlay(bin) when is_binary(bin) do
    case Jason.decode(bin) do
      {:ok, map} when is_map(map) -> require_overlay(map)
      _ -> {:error, :invalid_revision}
    end
  end

  defp require_overlay(_), do: {:error, :invalid_revision}

  # Mirror SQL headroom predicate (`CAST(... AS INTEGER)` + `COALESCE(..., 0)`).
  # Do not use `require_overlay/1` here — corrupt revision must not force
  # `:invalid_revision` when the real miss was a state/CAS race.
  defp overlay_lacks_clear_headroom?(%Room{} = room) do
    overlay_sql_style_revision(room.session_overlay) + 1 >= @revision_stride
  end

  defp overlay_sql_style_revision(overlay) when is_map(overlay) do
    case Map.get(overlay, "revision") || Map.get(overlay, :revision) do
      rev when is_integer(rev) ->
        rev

      rev when is_binary(rev) ->
        case Integer.parse(String.trim(rev)) do
          {n, ""} -> n
          _ -> 0
        end

      _ ->
        0
    end
  end

  defp overlay_sql_style_revision(bin) when is_binary(bin) do
    case Jason.decode(bin) do
      {:ok, map} when is_map(map) -> overlay_sql_style_revision(map)
      _ -> 0
    end
  end

  defp overlay_sql_style_revision(_), do: 0

  # Clear CAS missed: stride ceiling → :invalid_revision; otherwise CAS/state race → :conflict.
  defp miss_after_overlay_clear(room_id) do
    case get(room_id) do
      {:ok, room} ->
        if overlay_lacks_clear_headroom?(room) do
          {:error, :invalid_revision}
        else
          {:error, :conflict}
        end

      {:error, _} = err ->
        err
    end
  end

  defp scene_entries(scene) when is_map(scene) do
    case Map.get(scene, "entries") || Map.get(scene, :entries) do
      list when is_list(list) -> list
      _ -> []
    end
  end

  defp scene_entries(_), do: []

  defp reject_duplicate_blake3(entries, blake3) do
    if Enum.any?(entries, fn e -> entry_string(e, "blake3") == blake3 end) do
      {:error, :duplicate_blake3}
    else
      :ok
    end
  end

  defp put_overlay_entry(overlay, composed_entries, normalized, expected_revision) do
    next_id = next_entity_id(composed_entries)
    entry = Map.put(normalized, "entity_id", next_id)
    overlay_entries = scene_entries(overlay)

    {:ok,
     %{
       "revision" => expected_revision + 1,
       "entries" => overlay_entries ++ [entry]
     }}
  end

  defp fetch_room(room_id) do
    case Room.get_by_room_id(room_id) do
      {:ok, %Room{} = room} -> {:ok, room}
      {:ok, nil} -> {:error, :not_found}
      {:error, error} -> map_read_error(error, :append_scene_entry)
    end
  end

  defp normalize_mesh_entry(entry) do
    blake3 = entry_string(entry, "blake3") || entry_string(entry, :blake3)

    cond do
      not is_binary(blake3) or byte_size(blake3) != 64 ->
        {:error, :invalid_entry}

      not Regex.match?(~r/^[0-9a-f]{64}$/, blake3) ->
        {:error, :invalid_entry}

      true ->
        {:ok,
         %{
           "blake3" => blake3,
           "x" => entry_float(entry, "x", :x, 0.0),
           "y" => entry_float(entry, "y", :y, 1.5),
           "z" => entry_float(entry, "z", :z, 2.0),
           "qx" => entry_float(entry, "qx", :qx, 0.0),
           "qy" => entry_float(entry, "qy", :qy, 0.0),
           "qz" => entry_float(entry, "qz", :qz, 0.0),
           "qw" => entry_float(entry, "qw", :qw, 1.0),
           "sx" => entry_float(entry, "sx", :sx, 1.0),
           "sy" => entry_float(entry, "sy", :sy, 1.0),
           "sz" => entry_float(entry, "sz", :sz, 1.0),
           "r" => entry_float(entry, "r", :r, 0.85),
           "g" => entry_float(entry, "g", :g, 0.85),
           "b" => entry_float(entry, "b", :b, 0.9),
           "a" => entry_float(entry, "a", :a, 1.0),
           "half_extent" => 0.0,
           "flags" => 0
         }}
    end
  end

  defp build_appended_scene(scene, normalized, expected_revision) when is_map(scene) do
    entries = Map.get(scene, "entries") || Map.get(scene, :entries) || []
    append_normalized_entry(scene, entries, normalized, expected_revision)
  end

  defp append_normalized_entry(_scene, entries, _normalized, _expected_revision)
       when not is_list(entries) do
    {:error, :invalid_entry}
  end

  defp append_normalized_entry(scene, entries, normalized, expected_revision) do
    blake3 = normalized["blake3"]

    if Enum.any?(entries, fn e -> entry_string(e, "blake3") == blake3 end) do
      {:error, :duplicate_blake3}
    else
      put_appended_entry(scene, entries, normalized, expected_revision)
    end
  end

  defp put_appended_entry(scene, entries, normalized, expected_revision) do
    next_id = next_entity_id(entries)
    entry = Map.put(normalized, "entity_id", next_id)

    {:ok,
     %{
       "revision" => expected_revision + 1,
       "locomotion" => Map.get(scene, "locomotion") || Map.get(scene, :locomotion),
       "entries" => entries ++ [entry]
     }}
  end

  defp build_deleted_scene(scene, entity_id, expected_revision) when is_map(scene) do
    entries = Map.get(scene, "entries") || Map.get(scene, :entries) || []
    delete_mesh_entry(scene, entries, entity_id, expected_revision)
  end

  defp delete_mesh_entry(_scene, entries, _entity_id, _expected_revision)
       when not is_list(entries) do
    {:error, :entry_not_found}
  end

  defp delete_mesh_entry(scene, entries, entity_id, expected_revision) do
    # Exactly one matching entity_id; corrupt duplicates fail closed (x007b review).
    case Enum.split_with(entries, fn e -> entry_entity_id(e) == entity_id end) do
      {[], _} ->
        {:error, :entry_not_found}

      {[target], rest} ->
        if mesh_blake3?(target) do
          {:ok,
           %{
             "revision" => expected_revision + 1,
             "locomotion" => Map.get(scene, "locomotion") || Map.get(scene, :locomotion),
             "entries" => rest
           }}
        else
          {:error, :not_mesh_entry}
        end

      {_dupes, _rest} ->
        {:error, :ambiguous_entity_id}
    end
  end

  # Same charset as `normalize_mesh_entry/1` append path (lowercase hex only).
  defp mesh_blake3?(entry) when is_map(entry) do
    blake3 = entry_string(entry, "blake3") || entry_string(entry, :blake3)

    is_binary(blake3) and byte_size(blake3) == 64 and
      Regex.match?(~r/^[0-9a-f]{64}$/, blake3)
  end

  defp mesh_blake3?(_), do: false

  defp next_entity_id(entries) do
    max_content_id =
      entries
      |> Enum.map(&entry_entity_id/1)
      |> Enum.reject(&World.Protocol.user_root_band?/1)
      |> Enum.max(fn -> 0 end)

    # Prefer ids above demo boxes (10–18); never reuse.
    # If the candidate lands in the Presence user_root band, jump past it (x009b).
    candidate = max(max_content_id + 1, 100)

    if World.Protocol.user_root_band?(candidate) do
      World.Protocol.user_root_entity_after_band()
    else
      candidate
    end
  end

  defp entry_entity_id(e) do
    case Map.get(e, "entity_id") || Map.get(e, :entity_id) do
      id when is_integer(id) and id >= 0 -> id
      id when is_binary(id) -> parse_nonneg_int(id)
      _ -> 0
    end
  end

  defp parse_nonneg_int(id) do
    case Integer.parse(id) do
      {n, ""} when n >= 0 -> n
      _ -> 0
    end
  end

  defp entry_string(map, key) when is_map(map) do
    case Map.get(map, key) || Map.get(map, to_string(key)) do
      v when is_binary(v) -> v
      v when is_atom(v) -> Atom.to_string(v)
      v when is_integer(v) -> Integer.to_string(v)
      _ -> nil
    end
  end

  defp entry_float(map, bin_key, atom_key, default) do
    case Map.get(map, bin_key) || Map.get(map, atom_key) do
      v when is_float(v) ->
        v

      v when is_integer(v) ->
        v * 1.0

      v when is_binary(v) ->
        case Float.parse(v) do
          {f, _} -> f
          :error -> default
        end

      _ ->
        default
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
      cas_visibility_narrow_rotate(room, vis, @rotate_cas_attempts)
    else
      set_visibility_only(room, vis)
    end
  end

  defp set_visibility_only(%Room{} = room, vis) do
    case Room.set_visibility(room, %{visibility: vis}) do
      {:ok, %Room{visibility: updated_vis}} -> {:ok, updated_vis}
      {:error, error} -> storage_error(error, :set_visibility)
    end
  end

  @doc """
  Owner-driven Room key revoke (x007e / x00a0). Same effect as visibility narrow without
  changing visibility. HTTP: `POST …/room_key/rotate`.
  """
  @spec rotate_room_key(room_id(), owner_sub()) ::
          {:ok, non_neg_integer() | :noop}
          | {:error, :invalid_room_id | :not_found | :forbidden_host | storage_error()}
  def rotate_room_key(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    with {:ok, %Room{} = room} <- get(room_id),
         :ok <- owner_matches(room, sub) do
      do_rotate_room_key(room)
    end
  end

  def rotate_room_key(_, _), do: {:error, :invalid_room_id}

  @doc """
  x00a0: after auth friendship/follow revoke, rotate social-visibility rooms owned by
  either party. Returns list of `%{room_id:, room_key_epoch:}` (`room_key_epoch` may be `:noop`).
  """
  @spec rotate_for_relation_change(owner_sub(), owner_sub()) ::
          {:ok, [%{room_id: room_id(), room_key_epoch: non_neg_integer() | :noop}]}
          | {:error, storage_error()}
  def rotate_for_relation_change(sub_a, sub_b)
      when is_binary(sub_a) and sub_a != "" and is_binary(sub_b) and sub_b != "" do
    query =
      from r in Room,
        where: r.owner_sub in ^[sub_a, sub_b]

    try do
      rooms = World.Repo.all(query)

      rooms
      |> Enum.filter(&Visibility.needs_social?(&1.visibility))
      |> Enum.reduce_while([], &accumulate_relation_rotate/2)
      |> finalize_relation_rotate()
    rescue
      error -> storage_error(error, :rotate_for_relation_change)
    end
  end

  def rotate_for_relation_change(_, _), do: {:ok, []}

  defp accumulate_relation_rotate(room, acc) do
    case do_rotate_room_key(room) do
      {:ok, epoch} ->
        {:cont, [%{room_id: room.room_id, room_key_epoch: epoch} | acc]}

      {:error, :storage_error} ->
        {:halt, {:error, :storage_error}}

      {:error, _} ->
        {:cont, acc}
    end
  end

  defp finalize_relation_rotate({:error, _} = err), do: err
  defp finalize_relation_rotate(list) when is_list(list), do: {:ok, Enum.reverse(list)}

  @doc "Public lease view for HTTP session summaries (expired → dormant projection)."
  @spec lease_view_public(Room.t()) :: Room.t()
  def lease_view_public(%Room{} = room), do: lease_view(room)

  @doc """
  Catalog summaries visible to `viewer_sub` under RoomAccess (x007c).

  Does **not** include `room_key`, Token, `wire_cap`, or scene.
  `role_hint` is `owner` | `joinable` only (invisible rooms are omitted).

  Guarantees: owned rooms (up to owned limit) and public candidates are not
  dropped by a global top-N of all rooms before ACL.

  Non-guarantees: social/lan candidates are still “newest N by visibility class”
  then ACL-filtered — a friend’s rarely-updated room may fall outside the social
  scan window (friendship-graph-aware scan is out of scope).
  """
  @type catalog_summary :: %{
          room_id: room_id(),
          visibility: String.t(),
          owner_sub: owner_sub(),
          role_hint: String.t(),
          state: String.t(),
          active_host_sub: owner_sub() | nil
        }

  @host_lease_ttl_default_seconds 300

  # Per-bucket caps: owned + public + social(+lan) candidates, then ACL filter.
  # Avoids "global ORDER BY LIMIT then filter" dropping older visible private/friends rooms.
  @catalog_owned_limit 200
  @catalog_public_limit 200
  @catalog_social_limit 200
  @catalog_result_limit 200

  @spec list_visible_summaries(Plug.Conn.t(), owner_sub(), keyword()) ::
          {:ok, [catalog_summary()]} | {:error, storage_error()}
  def list_visible_summaries(conn, viewer_sub, opts \\ [])

  def list_visible_summaries(conn, viewer_sub, opts)
      when is_binary(viewer_sub) and viewer_sub != "" and is_list(opts) do
    mine_only? = Keyword.get(opts, :mine_only, false) == true
    include_lan? = not mine_only? and WorldWeb.ClientIp.lan_eligible?(conn)

    case list_catalog_rows(viewer_sub, mine_only?, include_lan?) do
      {:ok, rows} ->
        summaries =
          rows
          |> Enum.filter(fn row -> catalog_visible?(conn, viewer_sub, row) end)
          |> Enum.map(fn row -> catalog_summary(viewer_sub, row) end)
          |> Enum.take(@catalog_result_limit)

        {:ok, summaries}

      {:error, :storage_error} = err ->
        err
    end
  end

  def list_visible_summaries(_, _, _), do: {:error, :storage_error}

  defp list_catalog_rows(viewer_sub, true, _include_lan?) do
    case World.Repo.query(
           """
           SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at
           FROM rooms
           WHERE owner_sub = ?
           ORDER BY updated_at DESC
           LIMIT ?
           """,
           [viewer_sub, @catalog_owned_limit]
         ) do
      {:ok, %{rows: rows}} -> {:ok, Enum.map(rows, &catalog_row/1)}
      {:error, error} -> storage_error(error, :list_catalog_rows)
    end
  rescue
    e in [RuntimeError, ArgumentError] ->
      storage_error(e, :list_catalog_rows)
  end

  defp list_catalog_rows(viewer_sub, false, include_lan?) do
    social_vis =
      if include_lan? do
        ["friends", "friends_plus", "followers", "followers_plus", "lan"]
      else
        ["friends", "friends_plus", "followers", "followers_plus"]
      end

    # Union owned ∪ public ∪ social/lan candidates (dedupe by room_id), newest first.
    # x0080: non-mine catalog is live sessions only (active + unexpired lease).
    # Visibility ACL (incl. social S2S) still applied in catalog_visible?/3.
    placeholders = Enum.map_join(social_vis, ",", fn _ -> "?" end)
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    sql = """
    SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at FROM (
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 0 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE owner_sub = ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
      UNION
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 1 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE visibility = 'public' AND owner_sub != ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
      UNION
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 2 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE visibility IN (#{placeholders}) AND owner_sub != ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
    )
    ORDER BY pri ASC, updated_at DESC
    """

    params =
      List.flatten([
        viewer_sub,
        now,
        @catalog_owned_limit,
        viewer_sub,
        now,
        @catalog_public_limit,
        social_vis,
        viewer_sub,
        now,
        @catalog_social_limit
      ])

    case World.Repo.query(sql, params) do
      {:ok, %{rows: rows}} -> {:ok, Enum.map(rows, &catalog_row/1)}
      {:error, error} -> storage_error(error, :list_catalog_rows)
    end
  rescue
    e in [RuntimeError, ArgumentError] ->
      storage_error(e, :list_catalog_rows)
  end

  defp catalog_row([
         room_id,
         owner_sub,
         visibility,
         state,
         active_host_sub,
         host_lease_expires_at
       ])
       when is_binary(room_id) and is_binary(owner_sub) and is_binary(visibility) and
              is_binary(state) do
    %{
      room_id: room_id,
      owner_sub: owner_sub,
      visibility: visibility,
      state: state,
      active_host_sub: active_host_sub,
      host_lease_expires_at: host_lease_expires_at
    }
  end

  defp catalog_visible?(conn, viewer_sub, %{
         owner_sub: owner_sub,
         visibility: visibility
       }) do
    case World.RoomAccess.authorize(conn, owner_sub, viewer_sub, visibility) do
      :ok -> true
      # Fail closed for catalog: social outage / forbidden / lan → omit row.
      _ -> false
    end
  end

  defp catalog_summary(viewer_sub, row) do
    role_hint = if row.owner_sub == viewer_sub, do: "owner", else: "joinable"

    %{
      room_id: row.room_id,
      visibility: row.visibility,
      owner_sub: row.owner_sub,
      role_hint: role_hint,
      state: effective_catalog_state(row),
      active_host_sub: effective_catalog_active_host(row)
    }
  end

  defp effective_catalog_state(%{state: "active"} = row) do
    if lease_valid?(row.host_lease_expires_at), do: "active", else: "dormant"
  end

  defp effective_catalog_state(%{state: state}) when is_binary(state), do: state
  defp effective_catalog_state(_), do: "dormant"

  defp effective_catalog_active_host(%{state: "active"} = row) do
    if lease_valid?(row.host_lease_expires_at), do: row.active_host_sub, else: nil
  end

  defp effective_catalog_active_host(_), do: nil

  defp lease_valid?(%DateTime{} = expires_at) do
    DateTime.compare(expires_at, DateTime.utc_now()) == :gt
  end

  defp lease_valid?(%NaiveDateTime{} = expires_at) do
    expires_at
    |> DateTime.from_naive!("Etc/UTC")
    |> lease_valid?()
  end

  # Raw SQL catalog rows may surface ISO strings from SQLite.
  defp lease_valid?(expires_at) when is_binary(expires_at) do
    case DateTime.from_iso8601(expires_at) do
      {:ok, dt, _} ->
        lease_valid?(dt)

      _ ->
        case NaiveDateTime.from_iso8601(expires_at) do
          {:ok, ndt} -> lease_valid?(ndt)
          _ -> false
        end
    end
  end

  defp lease_valid?(_), do: false

  @doc """
  x0080: re-read room with **read-only** lease view (expired active looks dormant).

  Does not write DB — durable expiry is `expire_due_host_leases/0` (sweeper).
  """
  @spec get_session(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def get_session(room_id) when is_binary(room_id) do
    with {:ok, %Room{} = room} <- get(room_id) do
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
         {:ok, %Room{} = room} <- get(room_id),
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
         {:ok, %Room{}} <- get(room_id) do
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
         {:ok, %Room{} = room} <- get(room_id) do
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
         {:ok, %Room{} = room} <- get(room_id),
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
         {:ok, %Room{} = room} <- get(room_id),
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
         {:ok, %Room{} = room} <- get(room_id),
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
        {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
      {:ok, %{num_rows: 1}} -> get(room_id)
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
         {:ok, %Room{} = room} <- get(room_id) do
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
    case get(room_id) do
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

  defp owner_matches(%Room{owner_sub: owner}, sub) when owner == sub, do: :ok
  defp owner_matches(%Room{}, _), do: {:error, :forbidden_host}

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
  end

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

  # x009d: `editor_subs` column is JSON text `["sub", …]`.
  defp decode_editor_subs(nil), do: []

  defp decode_editor_subs(bin) when is_binary(bin) do
    case Jason.decode(bin) do
      {:ok, list} when is_list(list) ->
        Enum.filter(list, &(is_binary(&1) and &1 != ""))

      _ ->
        []
    end
  end

  defp decode_editor_subs(_), do: []

  @editor_subs_max 64

  defp encode_editor_subs(editors) when is_list(editors) do
    unique = Enum.uniq(editors)

    cond do
      length(unique) > @editor_subs_max ->
        {:error, :invalid_editors}

      Enum.all?(unique, &(is_binary(&1) and &1 != "")) ->
        {:ok, Jason.encode!(unique)}

      true ->
        {:error, :invalid_editors}
    end
  end

  defp encode_eligible_subs(subs) when is_list(subs) do
    case encode_editor_subs(subs) do
      {:ok, _} = ok -> ok
      {:error, :invalid_editors} -> {:error, :invalid_eligible}
    end
  end

  defp cas_visibility_narrow_rotate(_room, _vis, 0) do
    Logger.error("World.Rooms.set_visibility narrow+rotate CAS exhausted")
    {:error, :storage_error}
  end

  defp cas_visibility_narrow_rotate(%Room{} = room, vis, attempts_left) do
    case try_narrow_rotate_once(room, vis) do
      {:ok, _} = ok -> ok
      :conflict -> retry_narrow_after_conflict(room.room_id, vis, attempts_left - 1)
      {:error, error} -> storage_error(error, :set_visibility)
    end
  end

  defp try_narrow_rotate_once(%Room{} = room, vis) do
    expected_epoch = room.room_key_epoch
    new_key = generate_room_key_hex()
    new_epoch = expected_epoch + 1
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    prev_key = room.room_key
    prev_epoch = room.room_key_epoch

    sql = """
    UPDATE rooms
    SET visibility = ?, room_key = ?, room_key_epoch = ?, room_key_rotated_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND room_key_epoch = ?
    """

    case World.Repo.query(sql, [vis, new_key, new_epoch, now, room.room_id, expected_epoch]) do
      {:ok, %{num_rows: 1}} ->
        RoomKeyGrace.put(room.room_id, prev_key, prev_epoch)
        {:ok, vis}

      {:ok, %{num_rows: 0}} ->
        :conflict

      {:error, error} ->
        {:error, error}
    end
  end

  defp retry_narrow_after_conflict(room_id, vis, attempts_left) do
    case get(room_id) do
      {:ok, %Room{} = fresh} -> continue_narrow_after_conflict(fresh, vis, attempts_left)
      other -> other
    end
  end

  defp continue_narrow_after_conflict(%Room{} = fresh, vis, attempts_left) do
    cond do
      fresh.visibility == vis ->
        {:ok, vis}

      Visibility.narrower?(vis, fresh.visibility) ->
        cas_visibility_narrow_rotate(fresh, vis, attempts_left)

      true ->
        set_visibility_only(fresh, vis)
    end
  end

  # Owner / narrow: always retry on CAS miss. Periodic due: pass `due_cutoff:` so a
  # concurrent rotate that already refreshed `room_key_rotated_at` past the cutoff
  # does not bump another epoch (extra KeyStale).
  defp do_rotate_room_key(%Room{} = room, opts \\ []) do
    due_cutoff = Keyword.get(opts, :due_cutoff)
    do_rotate_room_key_cas(room, @rotate_cas_attempts, due_cutoff)
  end

  defp do_rotate_room_key_cas(_room, 0, _due_cutoff) do
    Logger.error("World.Rooms.rotate_room_key CAS exhausted")
    {:error, :storage_error}
  end

  defp do_rotate_room_key_cas(%Room{} = room, attempts_left, due_cutoff) do
    case try_rotate_once(room) do
      {:ok, _} = ok -> ok
      :conflict -> retry_rotate_after_conflict(room.room_id, attempts_left - 1, due_cutoff)
      {:error, error} -> storage_error(error, :rotate_room_key)
    end
  end

  defp try_rotate_once(%Room{} = room) do
    expected_epoch = room.room_key_epoch
    new_key = generate_room_key_hex()
    new_epoch = expected_epoch + 1
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    prev_key = room.room_key
    prev_epoch = room.room_key_epoch

    sql = """
    UPDATE rooms
    SET room_key = ?, room_key_epoch = ?, room_key_rotated_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND room_key_epoch = ?
    """

    case World.Repo.query(sql, [new_key, new_epoch, now, room.room_id, expected_epoch]) do
      {:ok, %{num_rows: 1}} ->
        RoomKeyGrace.put(room.room_id, prev_key, prev_epoch)
        {:ok, new_epoch}

      {:ok, %{num_rows: 0}} ->
        :conflict

      {:error, error} ->
        {:error, error}
    end
  end

  defp retry_rotate_after_conflict(room_id, attempts_left, due_cutoff) do
    case get(room_id) do
      {:ok, %Room{} = fresh} -> maybe_retry_rotate_cas(fresh, attempts_left, due_cutoff)
      other -> other
    end
  end

  defp maybe_retry_rotate_cas(%Room{} = fresh, attempts_left, due_cutoff) do
    if room_key_still_due_for_cas?(fresh, due_cutoff) do
      do_rotate_room_key_cas(fresh, attempts_left, due_cutoff)
    else
      # Concurrent rotate already satisfied the periodic window.
      {:ok, :noop}
    end
  end

  # `due_cutoff == nil` → owner / narrow revoke: always retry.
  defp room_key_still_due_for_cas?(_room, nil), do: true

  defp room_key_still_due_for_cas?(%Room{room_key_rotated_at: %DateTime{} = at}, cutoff) do
    DateTime.compare(at, cutoff) != :gt
  end

  defp room_key_still_due_for_cas?(%Room{room_key_rotated_at: nil}, _cutoff), do: false

  @doc """
  Rotate every room whose `room_key_rotated_at` is older than `hours` (x007e periodic).

  Returns `{rotated_count, errors}`. Hours `<= 0` skips (disabled).

  Null `room_key_rotated_at` is stamped to now **without** rotating (deploy baseline —
  do not treat `inserted_at` as due, which would KeyStale every pre-migration room at once).
  """
  @spec rotate_due_room_keys(pos_integer() | non_neg_integer()) ::
          {non_neg_integer(), non_neg_integer()}
  def rotate_due_room_keys(hours) when is_integer(hours) and hours <= 0, do: {0, 0}

  def rotate_due_room_keys(hours) when is_integer(hours) and hours > 0 do
    cutoff = DateTime.add(DateTime.utc_now(), -hours * 3600, :second)
    _ = stamp_null_room_key_rotated_at()

    case due_rooms_for_key_rotate(cutoff) do
      {:ok, rooms} ->
        Enum.reduce(rooms, {0, 0}, &accumulate_due_rotate(&1, &2, cutoff))

      {:error, _} ->
        {0, 1}
    end
  end

  defp stamp_null_room_key_rotated_at do
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    case World.Repo.query(
           """
           UPDATE rooms
           SET room_key_rotated_at = ?, updated_at = CURRENT_TIMESTAMP
           WHERE room_key_rotated_at IS NULL
           """,
           [now]
         ) do
      {:ok, _} ->
        :ok

      {:error, error} ->
        _ = storage_error(error, :stamp_room_key_rotated_at)
        :error
    end
  end

  defp due_rooms_for_key_rotate(cutoff) do
    sql = """
    SELECT room_id FROM rooms
    WHERE room_key_rotated_at IS NOT NULL
      AND room_key_rotated_at <= ?
    """

    case World.Repo.query(sql, [cutoff]) do
      {:ok, %{rows: rows}} ->
        {:ok, load_rooms_by_id_rows(rows)}

      {:error, error} ->
        storage_error(error, :rotate_due_room_keys)
    end
  end

  defp load_rooms_by_id_rows(rows) do
    Enum.flat_map(rows, &load_room_row/1)
  end

  defp load_room_row([room_id]) do
    case get(room_id) do
      {:ok, %Room{} = room} -> [room]
      _ -> []
    end
  end

  defp accumulate_due_rotate(room, {ok_n, err_n}, cutoff) do
    case do_rotate_room_key(room, due_cutoff: cutoff) do
      {:ok, :noop} -> {ok_n, err_n}
      {:ok, _epoch} -> {ok_n + 1, err_n}
      {:error, _} -> {ok_n, err_n + 1}
    end
  end

  # 16 random bytes → 32 hex chars; matches protocol `[A-Za-z0-9_-]+` / ≤64B.
  defp generate_wire_cap do
    Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
  end

  # x0069: 32 random bytes → 64 hex chars (Token JSON uses URL-safe base64 of raw bytes).
  def generate_room_key_hex do
    Base.encode16(:crypto.strong_rand_bytes(32), case: :lower)
  end

  @doc """
  Encode stored hex `room_key` to URL-safe base64 (no padding) for Token JSON.
  """
  def room_key_for_token(hex) when is_binary(hex) do
    case Base.decode16(hex, case: :lower) do
      {:ok, <<_::binary-size(32)>> = raw} ->
        {:ok, Base.url_encode64(raw, padding: false)}

      {:ok, _} ->
        {:error, :invalid_room_key}

      :error ->
        {:error, :invalid_room_key}
    end
  end

  defp map_create_error(error) do
    if unique_violation?(error) do
      {:error, :already_exists}
    else
      storage_error(error, :create)
    end
  end

  defp map_read_error(error, where) do
    if not_found_error?(error) do
      {:error, :not_found}
    else
      storage_error(error, where)
    end
  end

  defp storage_error(error, where) do
    Logger.error("World.Rooms.#{where} storage failure: #{Exception.message(error)}")
    {:error, :storage_error}
  end

  defp not_found_error?(error) do
    error
    |> ash_error_leaves()
    |> Enum.any?(fn
      %Ash.Error.Query.NotFound{} -> true
      _ -> false
    end) or Exception.message(error) =~ "record not found"
  end

  defp unique_violation?(error) do
    error
    |> ash_error_leaves()
    |> Enum.any?(&unique_leaf?/1)
  end

  defp ash_error_leaves(%{errors: errors}) when is_list(errors) do
    Enum.flat_map(errors, &ash_error_leaves/1)
  end

  defp ash_error_leaves(error), do: [error]

  defp unique_leaf?(%Ash.Error.Changes.InvalidChanges{fields: fields, message: message})
       when is_binary(message) do
    fields = List.wrap(fields)
    room_id_field?(fields) and taken_message?(message)
  end

  defp unique_leaf?(%Ash.Error.Changes.InvalidAttribute{field: :room_id, message: message})
       when is_binary(message) do
    taken_message?(message)
  end

  defp unique_leaf?(%{constraint: :unique_constraint, constraint_name: name})
       when is_binary(name) do
    room_id_constraint_name?(name)
  end

  defp unique_leaf?(%{constraint: :unique_constraint, fields: fields}) do
    room_id_field?(List.wrap(fields))
  end

  defp unique_leaf?(%{constraint_name: name}) when is_binary(name) do
    room_id_constraint_name?(name)
  end

  # SQLite primary-key conflict surfaces as a data-layer / unknown error message.
  defp unique_leaf?(error) do
    message = Exception.message(error)

    (String.contains?(message, "UNIQUE constraint failed") or
       String.contains?(message, "unique constraint")) and
      (String.contains?(message, "room_id") or String.contains?(message, "rooms"))
  end

  defp room_id_field?(fields), do: :room_id in fields

  defp room_id_constraint_name?(name) when is_binary(name) do
    String.contains?(name, "rooms") and
      (String.contains?(name, "room_id") or String.contains?(name, "unique") or
         String.contains?(name, "pkey"))
  end

  defp taken_message?(message) when is_binary(message) do
    String.contains?(message, "already been taken")
  end

  defp validate_room_id(room_id), do: World.RoomId.validate(room_id)
end
