defmodule World.Rooms.Scene do
  @moduledoc """
  Room scene definition: canonical scene + session overlay packed CAS (x00d6 split from `World.Rooms`).

  Guarantees: `append_scene_entry/4` / `delete_scene_entry/4` / overlay mutators /
  `save_overlay_to_canonical/3,4` are **atomic packed CAS** (`expected_canonical` +
  `expected_overlay` + revision bump) with SceneBudget enforcement (x00a2).
  **Canonical writer authorization is the caller's responsibility** (`World.Rooms.ensure_active_host/2`
  and `World.Rooms.ensure_scene_writer/2`); nothing here checks the caller identity.

  Non-guarantees: host lease / role checks, Room key rotation, catalog listing.

  Callers should use the `World.Rooms` facade; this module is the implementation unit.
  """

  import World.Rooms.Internal,
    only: [
      fetch_room: 2,
      map_read_error: 2,
      validate_room_id: 1,
      storage_error: 2,
      empty_overlay: 0,
      cleared_overlay_sql: 0,
      overlay_clear_headroom_sql: 0
    ]

  alias World.Rooms.Room

  @type room_id :: String.t()
  @type storage_error :: :storage_error

  @revision_stride World.Rooms.Internal.revision_stride()

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
         {:ok, %Room{} = room} <- fetch_room(room_id, :composed_scene) do
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
         {:ok, %Room{} = room} <- fetch_room(room_id, :append_overlay_entry),
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
           {:ok, fresh} <- fetch_room(room_id, :append_overlay_entry) do
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
         {:ok, %Room{} = room} <- fetch_room(room_id, :delete_overlay_entry),
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
           {:ok, fresh} <- fetch_room(room_id, :delete_overlay_entry) do
        compose_room_scene(fresh)
      end
    end
  end

  def delete_overlay_entry(_, _, _, _), do: {:error, :invalid_room_id}

  @doc """
  x00b7: merge `session_overlay` entries into canonical `scene` and clear overlay.

  **Auth:** not checked here. Callers must authorize active_host + scene writer
  (HTTP: `ensure_active_host` + `ensure_scene_writer`).

  Keeps overlay `entity_id`s. Empty overlay **and** empty patches is idempotent
  (returns composed, no rev bump). Non-empty `parents` / `fields` always CAS-write
  (x00b8). Atomic packed CAS updates both columns. Returns **composed** scene.
  """
  @spec save_overlay_to_canonical(room_id(), non_neg_integer(), non_neg_integer()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :invalid_entry
             | :parent_cycle
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | :scene_entries_exceeded
             | :scene_bytes_exceeded
             | storage_error()}
  def save_overlay_to_canonical(room_id, expected_canonical_revision, expected_overlay_revision)
      when is_binary(room_id) and is_integer(expected_canonical_revision) and
             expected_canonical_revision >= 0 and is_integer(expected_overlay_revision) and
             expected_overlay_revision >= 0 do
    save_overlay_to_canonical(
      room_id,
      expected_canonical_revision,
      expected_overlay_revision,
      %{}
    )
  end

  def save_overlay_to_canonical(_, _, _), do: {:error, :invalid_room_id}

  @spec save_overlay_to_canonical(room_id(), non_neg_integer(), non_neg_integer(), map()) ::
          {:ok, map()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :invalid_entry
             | :parent_cycle
             | :scene_revision_conflict
             | :invalid_scene_revision
             | :invalid_revision
             | :scene_entries_exceeded
             | :scene_bytes_exceeded
             | storage_error()}
  def save_overlay_to_canonical(
        room_id,
        expected_canonical_revision,
        expected_overlay_revision,
        patches
      )
      when is_binary(room_id) and is_integer(expected_canonical_revision) and
             expected_canonical_revision >= 0 and is_integer(expected_overlay_revision) and
             expected_overlay_revision >= 0 and is_map(patches) do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{} = room} <- fetch_room(room_id, :save_overlay_to_canonical),
         {:ok, overlay} <- require_overlay(room.session_overlay),
         {:ok, current_canon} <- scene_revision(room.scene),
         {:ok, current_overlay} <- scene_revision(overlay),
         :ok <- match_expected_revision(current_canon, expected_canonical_revision),
         :ok <- match_expected_revision(current_overlay, expected_overlay_revision),
         {:ok, normalized} <- normalize_save_patches(patches) do
      maybe_commit_save_canonical(
        room_id,
        room,
        overlay,
        normalized,
        expected_canonical_revision,
        expected_overlay_revision
      )
    end
  end

  def save_overlay_to_canonical(_, _, _, _), do: {:error, :invalid_room_id}

  defp maybe_commit_save_canonical(room_id, room, overlay, normalized, canon_exp, overlay_exp) do
    overlay_entries = scene_entries(overlay)

    if empty_save_fold?(overlay_entries, normalized) do
      compose_room_scene(room)
    else
      commit_save_canonical(room_id, room.scene, overlay_entries, normalized, %{
        canon: canon_exp,
        overlay: overlay_exp
      })
    end
  end

  defp empty_save_fold?(overlay_entries, %{
         parents: parents,
         fields: fields,
         spawns: spawns,
         despawns: despawns
       }) do
    overlay_entries == [] and parents == [] and fields == [] and spawns == [] and despawns == []
  end

  # patches + expected packed to keep arity ≤ 8 (Credo).
  defp commit_save_canonical(room_id, scene, overlay_entries, patches, expected) do
    %{parents: parents, fields: fields, spawns: spawns, despawns: despawns} = patches
    %{canon: canon_exp, overlay: overlay_exp} = expected

    with :ok <- ensure_revision_headroom(canon_exp),
         :ok <- ensure_revision_headroom(overlay_exp),
         {:ok, merged} <- build_merged_canonical_scene(scene, overlay_entries, canon_exp),
         # x00b9: despawn before spawn so same-blake3 replace in one Save works.
         {:ok, after_despawns} <- apply_save_despawns(scene_entries(merged), despawns),
         {:ok, after_spawns} <- apply_save_spawns(after_despawns, spawns),
         {:ok, patched_entries} <- apply_save_patches(after_spawns, parents, fields),
         next_scene = Map.put(merged, "entries", patched_entries),
         :ok <- cas_set_scene_and_cleared_overlay(room_id, canon_exp, overlay_exp, next_scene),
         {:ok, fresh} <- fetch_room(room_id, :save_overlay_to_canonical) do
      compose_room_scene(fresh)
    end
  end

  defp normalize_save_patches(patches) when is_map(patches) do
    parents_raw = Map.get(patches, "parents") || Map.get(patches, :parents) || []
    fields_raw = Map.get(patches, "fields") || Map.get(patches, :fields) || []
    spawns_raw = Map.get(patches, "spawns") || Map.get(patches, :spawns) || []
    despawns_raw = Map.get(patches, "despawns") || Map.get(patches, :despawns) || []

    with {:ok, parents} <- normalize_parent_patches(parents_raw),
         {:ok, fields} <- normalize_field_patches(fields_raw),
         {:ok, spawns} <- normalize_spawn_patches(spawns_raw),
         {:ok, despawns} <- normalize_despawn_patches(despawns_raw) do
      {:ok, %{parents: parents, fields: fields, spawns: spawns, despawns: despawns}}
    end
  end

  defp normalize_parent_patches(list) when is_list(list) do
    list
    |> Enum.reduce_while({:ok, []}, fn item, acc ->
      accumulate_normalized_patch(item, acc, &normalize_parent_patch/1)
    end)
    |> finish_patch_acc()
  end

  defp normalize_parent_patches(_), do: {:error, :invalid_entry}

  defp normalize_parent_patch(item) when is_map(item) do
    with {:ok, entity_id} <- parse_patch_u32(item, "entity_id", :entity_id),
         {:ok, parent_entity_id} <-
           parse_patch_u32(item, "parent_entity_id", :parent_entity_id) do
      {:ok, %{entity_id: entity_id, parent_entity_id: parent_entity_id}}
    end
  end

  defp normalize_parent_patch(_), do: {:error, :invalid_entry}

  defp normalize_field_patches(list) when is_list(list) do
    list
    |> Enum.reduce_while({:ok, []}, fn item, acc ->
      accumulate_normalized_patch(item, acc, &normalize_field_patch/1)
    end)
    |> finish_patch_acc()
  end

  defp normalize_field_patches(_), do: {:error, :invalid_entry}

  defp normalize_field_patch(item) when is_map(item) do
    with {:ok, entity_id} <- parse_patch_u32(item, "entity_id", :entity_id),
         {:ok, patch} <- put_optional_field_keys(%{entity_id: entity_id}, item) do
      if map_size(patch) == 1 do
        {:error, :invalid_entry}
      else
        {:ok, patch}
      end
    end
  end

  defp normalize_field_patch(_), do: {:error, :invalid_entry}

  # x00b9: host-allocated entity_id kept (no remap); collision → :invalid_entry at apply.
  defp normalize_spawn_patches(list) when is_list(list) do
    list
    |> Enum.reduce_while({:ok, []}, fn item, acc ->
      accumulate_normalized_patch(item, acc, &normalize_save_spawn_entry/1)
    end)
    |> finish_patch_acc()
  end

  defp normalize_spawn_patches(_), do: {:error, :invalid_entry}

  defp normalize_save_spawn_entry(item) when is_map(item) do
    with {:ok, entity_id} <- parse_patch_u32(item, "entity_id", :entity_id),
         :ok <- reject_user_root_spawn_id(entity_id),
         {:ok, base} <- normalize_mesh_entry(item),
         {:ok, flags} <- optional_spawn_u32(item, "flags", :flags, 0),
         {:ok, parent_entity_id} <-
           optional_spawn_u32(item, "parent_entity_id", :parent_entity_id, 0) do
      {:ok,
       base
       |> Map.put("entity_id", entity_id)
       |> Map.put("flags", flags)
       |> Map.put("parent_entity_id", parent_entity_id)
       |> Map.put("r", entry_float(item, "r", :r, base["r"]))
       |> Map.put("g", entry_float(item, "g", :g, base["g"]))
       |> Map.put("b", entry_float(item, "b", :b, base["b"]))
       |> Map.put("a", entry_float(item, "a", :a, base["a"]))}
    end
  end

  defp normalize_save_spawn_entry(_), do: {:error, :invalid_entry}

  defp reject_user_root_spawn_id(entity_id) do
    if World.Protocol.user_root_band?(entity_id) do
      {:error, :invalid_entry}
    else
      :ok
    end
  end

  defp optional_spawn_u32(item, str_key, atom_key, default) do
    case patch_map_get(item, str_key, atom_key) do
      :absent -> {:ok, default}
      v -> parse_u32_value(v)
    end
  end

  defp normalize_despawn_patches(list) when is_list(list) do
    list
    |> Enum.reduce_while({:ok, []}, fn item, acc ->
      case normalize_despawn_id(item) do
        {:ok, eid} ->
          {:ok, ids} = acc
          {:cont, {:ok, [eid | ids]}}

        {:error, _} = err ->
          {:halt, err}
      end
    end)
    |> finish_patch_acc()
  end

  defp normalize_despawn_patches(_), do: {:error, :invalid_entry}

  defp normalize_despawn_id(v) when is_integer(v), do: parse_u32_value(v)

  defp normalize_despawn_id(item) when is_map(item) do
    parse_patch_u32(item, "entity_id", :entity_id)
  end

  defp normalize_despawn_id(_), do: {:error, :invalid_entry}

  defp apply_save_spawns(entries, []), do: {:ok, entries}

  defp apply_save_spawns(entries, spawns) when is_list(entries) and is_list(spawns) do
    # Prepend on reversed base → O(n); final reverse restores order.
    case Enum.reduce_while(spawns, {:ok, Enum.reverse(entries)}, &append_save_spawn/2) do
      {:ok, rev} -> {:ok, Enum.reverse(rev)}
      {:error, _} = err -> err
    end
  end

  defp append_save_spawn(spawn, {:ok, acc}) do
    case reject_save_spawn_conflict(spawn, acc) do
      :ok -> {:cont, {:ok, [spawn | acc]}}
      {:error, _} = err -> {:halt, err}
    end
  end

  # Parent must already exist in acc (prior entries or earlier spawns in this batch).
  defp reject_save_spawn_conflict(spawn, acc) do
    eid = spawn["entity_id"]
    blake3 = spawn["blake3"]
    parent = spawn["parent_entity_id"] || 0

    cond do
      Enum.any?(acc, &entry_entity_id_eq?(&1, eid)) ->
        {:error, :invalid_entry}

      Enum.any?(acc, fn e -> entry_string(e, "blake3") == blake3 end) ->
        {:error, :duplicate_blake3}

      parent != 0 and is_nil(Enum.find(acc, &entry_entity_id_eq?(&1, parent))) ->
        {:error, :invalid_entry}

      true ->
        :ok
    end
  end

  defp apply_save_despawns(entries, []), do: {:ok, entries}

  defp apply_save_despawns(entries, despawns) when is_list(entries) and is_list(despawns) do
    Enum.reduce_while(despawns, {:ok, entries}, fn eid, {:ok, acc} ->
      case remove_mesh_entry_by_id(acc, eid) do
        {:ok, rest} -> {:cont, {:ok, rest}}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  # Save-time despawn: same match rules as delete_mesh_entry, no revision bump.
  defp remove_mesh_entry_by_id(entries, entity_id) do
    case Enum.split_with(entries, fn e -> entry_entity_id(e) == entity_id end) do
      {[], _} ->
        {:error, :invalid_entry}

      {[target], rest} ->
        if mesh_blake3?(target), do: {:ok, rest}, else: {:error, :invalid_entry}

      {_dupes, _rest} ->
        {:error, :invalid_entry}
    end
  end

  defp accumulate_normalized_patch(item, {:ok, acc}, normalize_fun) do
    case normalize_fun.(item) do
      {:ok, patch} -> {:cont, {:ok, [patch | acc]}}
      {:error, _} = err -> {:halt, err}
    end
  end

  defp finish_patch_acc({:ok, acc}), do: {:ok, Enum.reverse(acc)}
  defp finish_patch_acc({:error, _} = err), do: err

  defp put_optional_field_keys(patch, item) do
    with {:ok, patch} <- maybe_put_patch_u32(patch, item, "flags", :flags),
         {:ok, patch} <- maybe_put_patch_float(patch, item, "r", :r),
         {:ok, patch} <- maybe_put_patch_float(patch, item, "g", :g),
         {:ok, patch} <- maybe_put_patch_float(patch, item, "b", :b),
         {:ok, patch} <- maybe_put_patch_float(patch, item, "a", :a) do
      maybe_put_patch_blake3(patch, item)
    end
  end

  # Present-but-invalid keys → `:invalid_entry` (do not silently drop).
  defp maybe_put_patch_u32(patch, item, str_key, atom_key) do
    case patch_map_get(item, str_key, atom_key) do
      :absent ->
        {:ok, patch}

      v ->
        case parse_u32_value(v) do
          {:ok, n} -> {:ok, Map.put(patch, atom_key, n)}
          {:error, _} -> {:error, :invalid_entry}
        end
    end
  end

  defp maybe_put_patch_float(patch, item, str_key, atom_key) do
    case patch_map_get(item, str_key, atom_key) do
      :absent -> {:ok, patch}
      v when is_number(v) -> {:ok, Map.put(patch, atom_key, v * 1.0)}
      _ -> {:error, :invalid_entry}
    end
  end

  defp maybe_put_patch_blake3(patch, item) do
    case patch_map_get(item, "blake3", :blake3) do
      :absent ->
        {:ok, patch}

      blake3 when is_binary(blake3) and byte_size(blake3) == 64 ->
        lower = String.downcase(blake3)

        if Regex.match?(~r/^[0-9a-f]{64}$/, lower) do
          {:ok, Map.put(patch, :blake3, lower)}
        else
          {:error, :invalid_entry}
        end

      _ ->
        {:error, :invalid_entry}
    end
  end

  defp patch_map_get(item, str_key, atom_key) when is_map(item) do
    case Map.fetch(item, str_key) do
      {:ok, v} ->
        v

      :error ->
        case Map.fetch(item, atom_key) do
          {:ok, v} -> v
          :error -> :absent
        end
    end
  end

  defp parse_patch_u32(item, str_key, atom_key) do
    case Map.get(item, str_key) || Map.get(item, atom_key) do
      nil -> {:error, :invalid_entry}
      v -> parse_u32_value(v)
    end
  end

  defp parse_u32_value(v) when is_integer(v) and v >= 0 and v <= 0xFFFFFFFF, do: {:ok, v}
  defp parse_u32_value(_), do: {:error, :invalid_entry}

  defp apply_save_patches(entries, parents, fields) when is_list(entries) do
    with {:ok, after_parents} <- apply_parent_patches(entries, parents),
         {:ok, after_fields} <- apply_field_patches(after_parents, fields),
         :ok <- reject_parent_cycles(after_fields) do
      {:ok, after_fields}
    end
  end

  defp apply_parent_patches(entries, []), do: {:ok, entries}

  defp apply_parent_patches(entries, parents) do
    Enum.reduce_while(parents, {:ok, entries}, fn %{entity_id: eid, parent_entity_id: pid},
                                                  {:ok, acc} ->
      case patch_entry_parent(acc, eid, pid) do
        {:ok, next} -> {:cont, {:ok, next}}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp patch_entry_parent(entries, entity_id, parent_entity_id) do
    case Enum.find_index(entries, &entry_entity_id_eq?(&1, entity_id)) do
      nil ->
        {:error, :invalid_entry}

      idx ->
        if parent_entity_id != 0 and
             is_nil(Enum.find(entries, &entry_entity_id_eq?(&1, parent_entity_id))) do
          {:error, :invalid_entry}
        else
          entry = Enum.at(entries, idx)
          updated = put_entry_value(entry, "parent_entity_id", parent_entity_id)
          {:ok, List.replace_at(entries, idx, updated)}
        end
    end
  end

  defp apply_field_patches(entries, []), do: {:ok, entries}

  defp apply_field_patches(entries, fields) do
    Enum.reduce_while(fields, {:ok, entries}, fn patch, {:ok, acc} ->
      case patch_entry_fields(acc, patch) do
        {:ok, next} -> {:cont, {:ok, next}}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp patch_entry_fields(entries, %{entity_id: entity_id} = patch) do
    case Enum.find_index(entries, &entry_entity_id_eq?(&1, entity_id)) do
      nil ->
        {:error, :invalid_entry}

      idx ->
        entry =
          Enum.at(entries, idx)
          |> maybe_put_entry_key(patch, :flags, "flags")
          |> maybe_put_entry_key(patch, :r, "r")
          |> maybe_put_entry_key(patch, :g, "g")
          |> maybe_put_entry_key(patch, :b, "b")
          |> maybe_put_entry_key(patch, :a, "a")
          |> maybe_put_entry_key(patch, :blake3, "blake3")

        {:ok, List.replace_at(entries, idx, entry)}
    end
  end

  defp maybe_put_entry_key(entry, patch, atom_key, str_key) do
    case Map.fetch(patch, atom_key) do
      {:ok, v} -> put_entry_value(entry, str_key, v)
      :error -> entry
    end
  end

  defp put_entry_value(entry, key, value) when is_map(entry) do
    atom = String.to_existing_atom(key)

    cond do
      Map.has_key?(entry, key) -> Map.put(entry, key, value)
      Map.has_key?(entry, atom) -> Map.put(entry, atom, value)
      true -> Map.put(entry, key, value)
    end
  rescue
    ArgumentError -> Map.put(entry, key, value)
  end

  defp entry_entity_id_eq?(entry, entity_id) when is_map(entry) do
    case Map.get(entry, "entity_id") || Map.get(entry, :entity_id) do
      ^entity_id -> true
      _ -> false
    end
  end

  defp entry_entity_id_eq?(_, _), do: false

  defp entry_parent_id(entry) when is_map(entry) do
    case Map.get(entry, "parent_entity_id") || Map.get(entry, :parent_entity_id) do
      n when is_integer(n) and n >= 0 -> n
      _ -> 0
    end
  end

  defp reject_parent_cycles(entries) when is_list(entries) do
    parent_of =
      Map.new(entries, fn e ->
        eid = Map.get(e, "entity_id") || Map.get(e, :entity_id)
        {eid, entry_parent_id(e)}
      end)

    ids = MapSet.new(Map.keys(parent_of))

    Enum.reduce_while(Map.keys(parent_of), :ok, fn eid, :ok ->
      case walk_parent_chain(parent_of, eid, ids, MapSet.new()) do
        :ok -> {:cont, :ok}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp walk_parent_chain(_parent_of, 0, _ids, _seen), do: :ok

  defp walk_parent_chain(parent_of, entity_id, ids, seen) do
    cond do
      MapSet.member?(seen, entity_id) ->
        {:error, :parent_cycle}

      not MapSet.member?(ids, entity_id) ->
        # Dangling non-zero parent already rejected at patch time; treat as cycle-safe stop.
        :ok

      true ->
        parent = Map.get(parent_of, entity_id, 0)
        walk_parent_chain(parent_of, parent, ids, MapSet.put(seen, entity_id))
    end
  end

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
         {:ok, %Room{} = room} <- fetch_room(room_id, :append_scene_entry),
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
         {:ok, %Room{} = room} <- fetch_room(room_id, :delete_scene_entry),
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
    with {:ok, %Room{} = room} <- fetch_room(room_id, :cas_set_scene),
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
    with {:ok, %Room{} = room} <- fetch_room(room_id, :cas_set_overlay),
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

  # x00b7: canonical merge + overlay clear in one packed CAS.
  defp cas_set_scene_and_cleared_overlay(room_id, expected_canon, expected_overlay, next_scene)
       when is_binary(room_id) and is_integer(expected_canon) and is_integer(expected_overlay) and
              is_map(next_scene) do
    empty_overlay = %{"revision" => expected_overlay + 1, "entries" => []}

    with :ok <- World.SceneBudget.ensure_composed(next_scene, empty_overlay),
         {:ok, encoded_scene} <- World.SceneBudget.encode_checked(next_scene) do
      sql = """
      UPDATE rooms
      SET scene = ?,
          session_overlay = #{cleared_overlay_sql()},
          updated_at = CURRENT_TIMESTAMP
      WHERE room_id = ?
        AND CAST(json_extract(scene, '$.revision') AS INTEGER) = ?
        AND CAST(json_extract(session_overlay, '$.revision') AS INTEGER) = ?
        AND #{overlay_clear_headroom_sql()}
      """

      cas_packed_update_result(
        World.Repo.query(sql, [encoded_scene, room_id, expected_canon, expected_overlay]),
        room_id,
        :cas_set_scene_and_cleared_overlay
      )
    end
  end

  defp build_merged_canonical_scene(scene, overlay_entries, expected_revision)
       when is_map(scene) and is_list(overlay_entries) do
    {:ok,
     %{
       "revision" => expected_revision + 1,
       "locomotion" => Map.get(scene, "locomotion") || Map.get(scene, :locomotion),
       "entries" => scene_entries(scene) ++ overlay_entries
     }}
  end

  defp build_merged_canonical_scene(_, _, _), do: {:error, :invalid_entry}

  defp cas_packed_update_result({:ok, %{num_rows: 1}}, _room_id, _context), do: :ok

  defp cas_packed_update_result({:ok, %{num_rows: 0}}, room_id, context) do
    case fetch_room(room_id, context) do
      {:ok, _} -> {:error, :scene_revision_conflict}
      {:error, _} = err -> err
    end
  end

  defp cas_packed_update_result({:error, error}, _room_id, context),
    do: storage_error(error, context)

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
           "flags" => 0,
           "parent_entity_id" => 0
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
end
