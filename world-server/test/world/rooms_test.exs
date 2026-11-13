defmodule World.RoomsTest do
  use World.DataCase, async: false

  alias Ecto.Adapters.SQL.Sandbox
  alias World.Rooms
  alias World.Rooms.OwnerQuota
  alias World.Rooms.Room

  @owner "22222222-2222-2222-2222-222222222222"

  test "create persists owner and demo scene" do
    assert {:ok, "persist-a", "public", wire_cap} = Rooms.create("persist-a", @owner)
    assert is_binary(wire_cap) and wire_cap != ""
    assert {:ok, @owner} = Rooms.owner("persist-a")
    assert {:ok, scene} = Rooms.scene("persist-a")
    assert scene["revision"] == 1
    assert is_list(scene["entries"])
    assert length(scene["entries"]) == 14
  end

  test "create stores room_key hex and token encoding is 32 bytes (x0069)" do
    assert {:ok, "rk-room", _, _} = Rooms.create("rk-room", @owner)
    assert {:ok, %Room{room_key: hex, room_key_epoch: 0}} = Room.get_by_room_id("rk-room")
    assert byte_size(hex) == 64
    assert {:ok, b64} = Rooms.room_key_for_token(hex)
    assert {:ok, raw} = Base.url_decode64(b64, padding: false)
    assert byte_size(raw) == 32
  end

  test "visibility narrow rotates room_key epoch and parks prev in grace (x007e)" do
    alias World.Rooms.RoomKeyGrace

    assert {:ok, "rk-narrow", _, _} = Rooms.create("rk-narrow", @owner, "public")
    assert {:ok, %Room{room_key: key0, room_key_epoch: 0}} = Room.get_by_room_id("rk-narrow")

    assert {:ok, "friends"} = Rooms.set_visibility("rk-narrow", @owner, "friends")

    assert {:ok, %Room{room_key: key1, room_key_epoch: 1, visibility: "friends"}} =
             Room.get_by_room_id("rk-narrow")

    assert key1 != key0
    assert {:ok, %{prev_key: ^key0, prev_epoch: 0}} = RoomKeyGrace.get("rk-narrow")

    # Widen does not rotate.
    assert {:ok, "public"} = Rooms.set_visibility("rk-narrow", @owner, "public")
    assert {:ok, %Room{room_key: ^key1, room_key_epoch: 1}} = Room.get_by_room_id("rk-narrow")
  end

  test "accept_handover grants lease only to candidate (x00a1)" do
    owner = "a1a1a1a1-a1a1-a1a1-a1a1-a1a1a1a1a1a1"
    guest = "a2a2a2a2-a2a2-a2a2-a2a2-a2a2a2a2a2a2"
    other = "a3a3a3a3-a3a3-a3a3-a3a3-a3a3a3a3a3a3"

    assert {:ok, "ho-acc", _, _} = Rooms.create("ho-acc", owner)
    assert {:ok, _} = Rooms.activate_as_host("ho-acc", owner)
    assert {:ok, nominated} = Rooms.handover("ho-acc", owner, guest)
    assert nominated.state == "dormant"
    assert nominated.handover_candidate_sub == guest
    assert nominated.active_host_sub == nil

    assert {:error, :forbidden_host} = Rooms.accept_handover("ho-acc", other)
    assert {:ok, accepted} = Rooms.accept_handover("ho-acc", guest)
    assert accepted.state == "active"
    assert accepted.active_host_sub == guest
    assert accepted.handover_candidate_sub == nil
  end

  test "append_scene_entry rejects over entry budget (x00a2)" do
    owner = "b1b1b1b1-b1b1-b1b1-b1b1-b1b1b1b1b1b1"
    assert {:ok, "budget-ent", _, _} = Rooms.create("budget-ent", owner)
    assert {:ok, scene0} = Rooms.scene("budget-ent")

    # Raw SQL fill avoids AshSqlite json recursion on huge maps; CAS path still gates.
    entries =
      for i <- 1..World.SceneBudget.max_entries() do
        %{"entity_id" => i, "blake3" => String.duplicate("ab", 32)}
      end

    encoded =
      Jason.encode!(%{
        "revision" => scene0["revision"],
        "entries" => entries,
        "locomotion" => scene0["locomotion"] || %{}
      })

    assert {:ok, _} =
             World.Repo.query("UPDATE rooms SET scene = ? WHERE room_id = ?", [
               encoded,
               "budget-ent"
             ])

    assert {:ok, filled} = Rooms.scene("budget-ent")
    assert length(filled["entries"]) == World.SceneBudget.max_entries()

    assert {:error, :scene_entries_exceeded} =
             Rooms.append_scene_entry(
               "budget-ent",
               %{"blake3" => String.duplicate("cd", 32)},
               filled["revision"],
               0
             )
  end

  test "owner rotate_room_key bumps epoch without visibility change (x007e)" do
    alias World.Rooms.RoomKeyGrace

    assert {:ok, "rk-rot", _, _} = Rooms.create("rk-rot", @owner)

    assert {:ok, %Room{room_key: key0, room_key_epoch: 0, visibility: vis}} =
             Room.get_by_room_id("rk-rot")

    assert {:ok, 1} = Rooms.rotate_room_key("rk-rot", @owner)

    assert {:ok, %Room{room_key: key1, room_key_epoch: 1, visibility: ^vis}} =
             Room.get_by_room_id("rk-rot")

    assert key1 != key0
    assert {:ok, %{prev_key: ^key0, prev_epoch: 0}} = RoomKeyGrace.get("rk-rot")
    assert {:error, :forbidden_host} = Rooms.rotate_room_key("rk-rot", "other-sub")
  end

  test "rotate_due_room_keys rotates stale rooms (x007e periodic)" do
    assert {:ok, "rk-due", _, _} = Rooms.create("rk-due", @owner)
    assert {:ok, %Room{room_key_epoch: 0} = room} = Room.get_by_room_id("rk-due")
    stale = DateTime.add(DateTime.utc_now(), -48 * 3600, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             Room.rotate_room_key(room, %{
               room_key: room.room_key,
               room_key_epoch: room.room_key_epoch,
               room_key_rotated_at: stale
             })

    assert {0, 0} = Rooms.rotate_due_room_keys(0)
    {ok_n, err_n} = Rooms.rotate_due_room_keys(24)
    assert err_n == 0
    assert ok_n >= 1
    assert {:ok, %Room{room_key_epoch: epoch}} = Room.get_by_room_id("rk-due")
    assert epoch >= 1
  end

  test "rotate_due stamps null room_key_rotated_at without rotating (x007e deploy baseline)" do
    assert {:ok, "rk-stamp", _, _} = Rooms.create("rk-stamp", @owner)
    assert {:ok, %Room{room_key_epoch: 0} = room} = Room.get_by_room_id("rk-stamp")

    # Simulate pre-migration / null column without using inserted_at as due.
    assert {:ok, _} =
             Room.rotate_room_key(room, %{
               room_key: room.room_key,
               room_key_epoch: room.room_key_epoch,
               room_key_rotated_at: nil
             })

    {ok_n, err_n} = Rooms.rotate_due_room_keys(24)
    assert err_n == 0
    assert ok_n == 0

    assert {:ok, %Room{room_key_epoch: 0, room_key_rotated_at: %DateTime{}}} =
             Room.get_by_room_id("rk-stamp")
  end

  test "create rejects duplicate room_id" do
    assert {:ok, _, _, _} = Rooms.create("dup-room", @owner)
    assert {:error, :already_exists} = Rooms.create("dup-room", @owner)
  end

  test "create rejects invalid room_id" do
    assert {:error, :invalid_room_id} = Rooms.create("bad/id", @owner)
    assert {:error, :missing_owner} = Rooms.create("ok-id", "")
  end

  test "create rejects when owner room cap is exceeded (x0071 / x0097)" do
    # Unique owner avoids leftover room_owner_quotas rows outside the sandbox TX.
    owner = "cap00000-0000-0000-0000-000000000001"
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 1)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    assert {:ok, "cap-1", _, _} = Rooms.create("cap-1", owner)
    assert {:error, :owner_room_cap_exceeded} = Rooms.create("cap-2", owner)

    assert %OwnerQuota{room_count: 1} =
             Repo.get(OwnerQuota, owner)
  end

  test "conditional quota: dual create callers yield exactly one success (x0097)" do
    # Shared sandbox (`async: false`) serializes Tasks onto one connection —
    # asserts the API contract (exactly one success under cap=1), not a true
    # multi-connection race. Atomic UPDATE is covered by the bump-at-cap test.
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 1)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    owner = "33333333-3333-3333-3333-333333333333"

    results =
      1..2
      |> Enum.map(fn i ->
        Task.async(fn -> Rooms.create("race-cap-#{i}", owner) end)
      end)
      |> Enum.map(&Task.await(&1, 10_000))

    oks = Enum.count(results, &match?({:ok, _, _, _}, &1))
    errs = Enum.count(results, &match?({:error, :owner_room_cap_exceeded}, &1))
    assert oks == 1
    assert errs == 1

    assert %OwnerQuota{room_count: 1} =
             Repo.get(OwnerQuota, owner)

    {:ok, %{rows: [[count]]}} =
      Repo.query("SELECT COUNT(*) FROM rooms WHERE owner_sub = ?", [owner])

    assert count == 1
  end

  test "conditional UPDATE refuses second bump at cap (x0097)" do
    # Direct ledger probe: the overshoot gate is `room_count < cap`, independent
    # of sandbox Task serialization around Rooms.create/3.
    owner = "55555555-5555-5555-5555-555555555555"
    cap = 1

    assert {:ok, _} =
             %OwnerQuota{}
             |> OwnerQuota.changeset(%{owner_sub: owner, room_count: 0})
             |> Repo.insert()

    {1, _} =
      from(q in OwnerQuota,
        where: q.owner_sub == ^owner and q.room_count < ^cap
      )
      |> Repo.update_all(inc: [room_count: 1])

    {0, _} =
      from(q in OwnerQuota,
        where: q.owner_sub == ^owner and q.room_count < ^cap
      )
      |> Repo.update_all(inc: [room_count: 1])

    assert %OwnerQuota{room_count: 1} =
             Repo.get(OwnerQuota, owner)
  end

  test "duplicate create rolls back quota bump (x0097)" do
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 2)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    owner = "44444444-4444-4444-4444-444444444444"

    assert {:ok, "dup-quota", _, _} = Rooms.create("dup-quota", owner)
    assert {:error, :already_exists} = Rooms.create("dup-quota", owner)

    assert %OwnerQuota{room_count: 1} =
             Repo.get(OwnerQuota, owner)

    assert {:ok, "dup-quota-2", _, _} = Rooms.create("dup-quota-2", owner)
    assert {:error, :owner_room_cap_exceeded} = Rooms.create("dup-quota-3", owner)
  end

  test "create returns storage_error when max_rooms_per_owner is invalid (x0071)" do
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 0)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    assert {:error, :storage_error} = Rooms.create("bad-cap-room", @owner)
  end

  test "require_existing and scene return not_found for missing rooms" do
    assert {:error, :not_found} = Rooms.require_existing("ghost-room")
    assert {:error, :not_found} = Rooms.scene("ghost-room")
    assert {:error, :not_found} = Rooms.owner("ghost-room")
    assert {:error, :not_found} = Rooms.ensure_owner("ghost-room", @owner)
  end

  test "ensure_owner distinguishes forbidden from not_found" do
    assert {:ok, _, _, _} = Rooms.create("owned-room", @owner)
    assert :ok = Rooms.ensure_owner("owned-room", @owner)
    assert {:error, :forbidden_host} = Rooms.ensure_owner("owned-room", "other-sub")
  end

  test "room survives beyond in-memory process lifetime (DB read)" do
    assert {:ok, "survive", _, _} = Rooms.create("survive", @owner)

    # Direct Ash read proves the row is in SQLite, not a GenServer map.
    assert {:ok, %Room{owner_sub: @owner, scene: scene}} = Room.get_by_room_id("survive")
    assert scene["locomotion"]["mode"] == "noclip"
  end

  test "duplicate create maps to already_exists via primary key" do
    assert {:ok, _, _, _} = Rooms.create("pk-dup", @owner)
    # Bypass ensure_absent-style races: second Ash insert hits PK uniqueness.
    assert {:error, :already_exists} = Rooms.create("pk-dup", @owner)
  end

  test "claim_host_generation reuses same pub and bumps on handoff (x003a)" do
    assert {:ok, "gen-room", _, _} = Rooms.create("gen-room", @owner)
    pub_a = :crypto.strong_rand_bytes(32)
    pub_b = :crypto.strong_rand_bytes(32)

    assert {:ok, 1} = Rooms.claim_host_generation("gen-room", pub_a)
    assert {:ok, 1} = Rooms.claim_host_generation("gen-room", pub_a)
    assert {:ok, 2} = Rooms.claim_host_generation("gen-room", pub_b)
    assert {:ok, 2} = Rooms.claim_host_generation("gen-room", pub_b)

    assert :ok = Rooms.match_claimed_host("gen-room", pub_b, 2)
    assert {:error, :stale_host_attestation} = Rooms.match_claimed_host("gen-room", pub_a, 1)
  end

  test "claim_host_generation CAS rejects stale expected row (x003a dual-host race)" do
    assert {:ok, "cas-room", _, _} = Rooms.create("cas-room", @owner)
    pub_a = :crypto.strong_rand_bytes(32)
    pub_b = :crypto.strong_rand_bytes(32)
    assert {:ok, 1} = Rooms.claim_host_generation("cas-room", pub_a)

    # Lost race: UPDATE still expecting gen=0 / nil pub must not overwrite gen=1.
    encoded_b = Base.url_encode64(pub_b, padding: false)

    sql = """
    UPDATE rooms
    SET host_generation = 99, attested_host_pub = ?, updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ? AND host_generation = 0 AND attested_host_pub IS NULL
    """

    assert {:ok, %{num_rows: 0}} = World.Repo.query(sql, [encoded_b, "cas-room"])
    assert {:ok, 1} = Rooms.claim_host_generation("cas-room", pub_a)
    assert {:ok, 2} = Rooms.claim_host_generation("cas-room", pub_b)
  end

  test "create and reads return storage_error when repo is down" do
    assert {:ok, _, _, _} = Rooms.create("before-down", @owner)
    stop_repo!()

    assert {:error, :storage_error} = Rooms.create("during-down", @owner)
    assert {:error, :storage_error} = Rooms.require_existing("before-down")
    assert {:error, :storage_error} = Rooms.owner("before-down")
    assert {:error, :storage_error} = Rooms.ensure_owner("before-down", @owner)
    assert {:error, :storage_error} = Rooms.scene("before-down")
  after
    restart_repo!()
  end

  test "host lease expiry dormants session (x0080)" do
    assert {:ok, "lease-1", _, _} = Rooms.create("lease-1", @owner)
    assert {:ok, %Room{state: "dormant"}} = Rooms.get("lease-1")

    assert {:ok, %Room{state: "active", active_host_sub: @owner}} =
             Rooms.activate_as_host("lease-1", @owner)

    stale = DateTime.add(DateTime.utc_now(), -10, :second) |> DateTime.truncate(:second)

    assert {:ok, %{num_rows: 1}} =
             World.Repo.query(
               """
               UPDATE rooms
               SET host_lease_expires_at = ?
               WHERE room_id = ?
               """,
               [stale, "lease-1"]
             )

    assert {1, 0} = Rooms.expire_due_host_leases()
    assert {:ok, %Room{state: "dormant", active_host_sub: nil}} = Rooms.get("lease-1")
  end

  test "handover and takeover CAS (x0080 / x00a1)" do
    guest = "88888888-8888-8888-8888-888888888888"
    assert {:ok, "ht-1", _, _} = Rooms.create("ht-1", @owner)
    assert {:ok, _} = Rooms.activate_as_host("ht-1", @owner)

    assert {:ok,
            %Room{
              active_host_sub: nil,
              state: "dormant",
              handover_candidate_sub: ^guest,
              owner_sub: @owner
            }} = Rooms.handover("ht-1", @owner, guest)

    # Pending candidate blocks takeover / owner host-token claim (x00a1).
    assert {:ok, _} = Rooms.set_eligible_host_subs("ht-1", @owner, [guest])
    assert {:error, :conflict} = Rooms.takeover("ht-1", guest)
    assert {:error, :forbidden_host} = Rooms.activate_as_host("ht-1", @owner)

    assert {:ok, %Room{active_host_sub: ^guest, state: "active", handover_candidate_sub: nil}} =
             Rooms.accept_handover("ht-1", guest)

    assert {:error, :conflict} = Rooms.takeover("ht-1", @owner)
    assert {:ok, _} = Rooms.handover("ht-1", guest, nil)
    assert {:ok, %Room{active_host_sub: ^guest, state: "active"}} = Rooms.takeover("ht-1", guest)
  end

  test "lease_view hides expired handover candidate (x00a1)" do
    owner = "e1e1e1e1-e1e1-e1e1-e1e1-e1e1e1e1e1e1"
    guest = "e2e2e2e2-e2e2-e2e2-e2e2-e2e2e2e2e2e2"
    assert {:ok, "ho-exp", _, _} = Rooms.create("ho-exp", owner)
    assert {:ok, _} = Rooms.activate_as_host("ho-exp", owner)
    assert {:ok, _} = Rooms.handover("ho-exp", owner, guest)

    past = DateTime.utc_now() |> DateTime.add(-60, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             World.Repo.query(
               """
               UPDATE rooms
               SET handover_candidate_expires_at = ?
               WHERE room_id = ?
               """,
               [past, "ho-exp"]
             )

    {:ok, raw} = Rooms.get("ho-exp")
    viewed = Rooms.lease_view_public(raw)
    assert viewed.handover_candidate_sub == nil
    assert viewed.handover_candidate_expires_at == nil
  end

  test "expire_due_host_leases durable-clears expired handover candidate (x00a1)" do
    owner = "e3e3e3e3-e3e3-e3e3-e3e3-e3e3e3e3e3e3"
    guest = "e4e4e4e4-e4e4-e4e4-e4e4-e4e4e4e4e4e4"
    assert {:ok, "ho-sw", _, _} = Rooms.create("ho-sw", owner)
    assert {:ok, _} = Rooms.activate_as_host("ho-sw", owner)
    assert {:ok, _} = Rooms.handover("ho-sw", owner, guest)

    past = DateTime.utc_now() |> DateTime.add(-60, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             World.Repo.query(
               """
               UPDATE rooms
               SET handover_candidate_expires_at = ?
               WHERE room_id = ?
               """,
               [past, "ho-sw"]
             )

    assert {_, _} = Rooms.expire_due_host_leases()

    assert {:ok, %Room{handover_candidate_sub: nil, handover_candidate_expires_at: nil}} =
             Rooms.get("ho-sw")
  end

  test "owner may re-nominate pending handover candidate (x00a1)" do
    owner = "e5e5e5e5-e5e5-e5e5-e5e5-e5e5e5e5e5e5"
    guest_a = "e6e6e6e6-e6e6-e6e6-e6e6-e6e6e6e6e6e6"
    guest_b = "e7e7e7e7-e7e7-e7e7-e7e7-e7e7e7e7e7e7"
    assert {:ok, "ho-ren", _, _} = Rooms.create("ho-ren", owner)
    assert {:ok, _} = Rooms.activate_as_host("ho-ren", owner)
    assert {:ok, first} = Rooms.handover("ho-ren", owner, guest_a)
    assert first.handover_candidate_sub == guest_a
    assert {:ok, second} = Rooms.handover("ho-ren", owner, guest_b)
    assert second.handover_candidate_sub == guest_b
    assert second.state == "dormant"
  end

  test "canonical append rejects when composed with overlay would exceed budget (x00a2)" do
    owner = "e8e8e8e8-e8e8-e8e8-e8e8-e8e8e8e8e8e8"
    assert {:ok, "budget-comp", _, _} = Rooms.create("budget-comp", owner)
    assert {:ok, scene0} = Rooms.scene("budget-comp")
    canon_n = World.SceneBudget.max_entries() - 1

    canon_entries =
      for i <- 1..canon_n do
        %{"entity_id" => i, "blake3" => String.duplicate("ab", 32)}
      end

    overlay_entries = [
      %{"entity_id" => 90_001, "blake3" => String.duplicate("cd", 32)},
      %{"entity_id" => 90_002, "blake3" => String.duplicate("ef", 32)}
    ]

    assert {:ok, _} =
             World.Repo.query(
               "UPDATE rooms SET scene = ?, session_overlay = ? WHERE room_id = ?",
               [
                 Jason.encode!(%{
                   "revision" => scene0["revision"],
                   "entries" => canon_entries,
                   "locomotion" => scene0["locomotion"] || %{}
                 }),
                 Jason.encode!(%{"revision" => 0, "entries" => overlay_entries}),
                 "budget-comp"
               ]
             )

    assert {:ok, filled} = Rooms.scene("budget-comp")

    assert {:error, :scene_entries_exceeded} =
             Rooms.append_scene_entry(
               "budget-comp",
               %{"blake3" => String.duplicate("11", 32)},
               filled["revision"],
               0
             )
  end

  test "owner Host Token activate cannot overwrite live takeover host (x0080)" do
    guest = "98989898-9898-9898-9898-989898989898"
    assert {:ok, "race-1", _, _} = Rooms.create("race-1", @owner)
    assert {:ok, %Room{state: "dormant"}} = Rooms.get("race-1")
    assert {:ok, _} = Rooms.set_eligible_host_subs("race-1", @owner, [guest])
    assert {:ok, _} = Rooms.takeover("race-1", guest)
    # Full path: authorize fails once takeover committed.
    assert {:error, :forbidden_host} = Rooms.activate_as_host("race-1", @owner)
    # TOCTOU window after authorize-OK on dormant: CAS alone must not steal live host.
    assert {:error, :conflict} = Rooms.activate_as_host_cas("race-1", @owner)
    assert {:ok, %Room{active_host_sub: ^guest, state: "active"}} = Rooms.get("race-1")
  end

  test "renew_host_lease CAS conflicts when active_host was stolen (x0080)" do
    guest = "c9c9c9c9-c9c9-c9c9-c9c9-c9c9c9c9c9c9"
    assert {:ok, "race-2", _, _} = Rooms.create("race-2", @owner)
    assert {:ok, _} = Rooms.activate_as_host("race-2", @owner)

    # Simulate TOCTOU: another host claimed between authorize-view and renew UPDATE.
    assert {:ok, %{num_rows: 1}} =
             World.Repo.query(
               """
               UPDATE rooms
               SET active_host_sub = ?
               WHERE room_id = ? AND state = 'active'
               """,
               [guest, "race-2"]
             )

    assert {:error, :conflict} = Rooms.renew_host_lease("race-2", @owner)
    assert {:ok, %Room{active_host_sub: ^guest}} = Rooms.get("race-2")
  end

  defp stop_repo! do
    case Supervisor.terminate_child(World.Supervisor, World.Repo) do
      :ok -> :ok
      {:error, :not_found} -> :ok
    end
  end

  defp restart_repo! do
    case Supervisor.restart_child(World.Supervisor, World.Repo) do
      {:ok, _} -> :ok
      {:ok, _, _} -> :ok
      {:error, {:already_started, _}} -> :ok
      {:error, :running} -> :ok
      other -> flunk("failed to restart World.Repo: #{inspect(other)}")
    end

    # x003b: Repo restart leaves Sandbox in auto; restore manual for later DataCase owners.
    Sandbox.mode(World.Repo, :manual)
  end
end
