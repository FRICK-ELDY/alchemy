defmodule World.SessionOverlayTest do
  @moduledoc "x009e: session_overlay compose / dormant clear / eligible takeover"
  use World.DataCase, async: false

  alias World.Rooms

  @owner "11111111-1111-1111-1111-111111111111"
  @guest "22222222-2222-2222-2222-222222222222"
  @blake3 String.duplicate("aa", 32)

  test "overlay append is visible in composed_scene and cleared on dormant with rev bump" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-1", @owner)
    assert {:ok, composed0} = Rooms.composed_scene(room_id)
    packed0 = composed0["revision"]
    assert {:ok, canon0, 0} = Rooms.unpack_revision(packed0)

    # create is dormant → first activate clears+bumps overlay 0→1.
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, after_activate} = Rooms.composed_scene(room_id)

    assert {:ok, ^canon0, overlay_after_activate} =
             Rooms.unpack_revision(after_activate["revision"])

    assert {:ok, composed1} =
             Rooms.append_overlay_entry(
               room_id,
               %{"blake3" => @blake3},
               canon0,
               overlay_after_activate
             )

    assert Enum.any?(composed1["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, overlay1} = Rooms.unpack_revision(composed1["revision"])
    assert overlay1 == overlay_after_activate + 1

    assert {:ok, _} = Rooms.handover(room_id, @owner, nil)
    assert {:ok, after_dormant} = Rooms.composed_scene(room_id)
    refute Enum.any?(after_dormant["entries"], fn e -> e["blake3"] == @blake3 end)
    # Clear bumps overlay revision so packed GenRev never regresses.
    assert {:ok, ^canon0, overlay_dormant} = Rooms.unpack_revision(after_dormant["revision"])
    assert overlay_dormant == overlay1 + 1
    assert after_dormant["revision"] > composed1["revision"]
  end

  test "takeover requires eligible_host_subs (owner always ok)" do
    assert {:ok, room_id, _, _} = Rooms.create("to-elig", @owner)

    assert {:error, :forbidden_host} = Rooms.takeover(room_id, @guest)

    assert {:ok, _} = Rooms.set_eligible_host_subs(room_id, @owner, [@guest])
    assert {:ok, updated} = Rooms.takeover(room_id, @guest)
    assert updated.active_host_sub == @guest
    assert updated.state == "active"
  end

  test "lease-expired takeover clears prior host overlay without sweeper" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-lease-to", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, composed0} = Rooms.composed_scene(room_id)
    assert {:ok, canon0, overlay0} = Rooms.unpack_revision(composed0["revision"])

    assert {:ok, composed1} =
             Rooms.append_overlay_entry(room_id, %{"blake3" => @blake3}, canon0, overlay0)

    assert Enum.any?(composed1["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, overlay1} = Rooms.unpack_revision(composed1["revision"])

    past = DateTime.utc_now() |> DateTime.add(-60, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             World.Repo.query(
               "UPDATE rooms SET host_lease_expires_at = ? WHERE room_id = ?",
               [past, room_id]
             )

    assert {:ok, _} = Rooms.set_eligible_host_subs(room_id, @owner, [@guest])
    assert {:ok, _} = Rooms.takeover(room_id, @guest)

    assert {:ok, composed_after} = Rooms.composed_scene(room_id)
    refute Enum.any?(composed_after["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, overlay_after} = Rooms.unpack_revision(composed_after["revision"])
    assert overlay_after == overlay1 + 1
  end

  test "lease-expired activate_as_host clears overlay; self-renew keeps it" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-lease-act", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, composed0} = Rooms.composed_scene(room_id)
    assert {:ok, canon0, overlay0} = Rooms.unpack_revision(composed0["revision"])

    assert {:ok, composed1} =
             Rooms.append_overlay_entry(room_id, %{"blake3" => @blake3}, canon0, overlay0)

    assert {:ok, ^canon0, overlay1} = Rooms.unpack_revision(composed1["revision"])

    # Self renew while lease valid keeps overlay.
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, still} = Rooms.composed_scene(room_id)
    assert Enum.any?(still["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, ^overlay1} = Rooms.unpack_revision(still["revision"])

    past = DateTime.utc_now() |> DateTime.add(-60, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             World.Repo.query(
               "UPDATE rooms SET host_lease_expires_at = ? WHERE room_id = ?",
               [past, room_id]
             )

    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, composed_after} = Rooms.composed_scene(room_id)
    refute Enum.any?(composed_after["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, overlay_after} = Rooms.unpack_revision(composed_after["revision"])
    assert overlay_after == overlay1 + 1
  end

  test "nominate handover clears overlay (host identity change)" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-nominate", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)
    assert {:ok, composed0} = Rooms.composed_scene(room_id)
    assert {:ok, canon0, overlay0} = Rooms.unpack_revision(composed0["revision"])

    assert {:ok, composed1} =
             Rooms.append_overlay_entry(room_id, %{"blake3" => @blake3}, canon0, overlay0)

    assert {:ok, ^canon0, overlay1} = Rooms.unpack_revision(composed1["revision"])

    assert {:ok, _} = Rooms.handover(room_id, @owner, @guest)
    assert {:ok, composed_after} = Rooms.composed_scene(room_id)
    refute Enum.any?(composed_after["entries"], fn e -> e["blake3"] == @blake3 end)
    assert {:ok, ^canon0, overlay_after} = Rooms.unpack_revision(composed_after["revision"])
    assert overlay_after == overlay1 + 1
  end

  test "pack_revision rejects components at or above stride" do
    assert {:error, :invalid_revision} = Rooms.pack_revision(1_000_000, 0)
    assert {:error, :invalid_revision} = Rooms.pack_revision(0, 1_000_000)
    assert {:ok, _} = Rooms.pack_revision(999_999, 999_999)
  end

  test "compose fail-closed on overflow ledger; clear fail-closed at stride ceiling" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-stride", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)

    overflow = %{"revision" => 1_000_000, "entries" => []}

    assert {:ok, _} =
             World.Repo.query("UPDATE rooms SET session_overlay = ? WHERE room_id = ?", [
               Jason.encode!(overflow),
               room_id
             ])

    assert {:error, :invalid_revision} = Rooms.composed_scene(room_id)

    ceiling = %{"revision" => 999_999, "entries" => []}

    assert {:ok, _} =
             World.Repo.query("UPDATE rooms SET session_overlay = ? WHERE room_id = ?", [
               Jason.encode!(ceiling),
               room_id
             ])

    assert {:ok, _} = Rooms.composed_scene(room_id)
    assert {:error, :invalid_revision} = Rooms.handover(room_id, @owner, nil)
  end

  test "compose fails closed on non-integer scene revision" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-corrupt-rev", @owner)
    assert {:ok, room} = Rooms.get(room_id)
    corrupt = Map.put(room.scene, "revision", "not-an-int")
    assert {:ok, _} = Rooms.Room.set_scene(room, %{scene: corrupt})
    assert {:error, :invalid_revision} = Rooms.composed_scene(room_id)
  end

  test "compose and overlay mutate fail-closed on non-integer overlay revision" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-bad-rev", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)

    bad = %{"revision" => "x", "entries" => []}

    assert {:ok, _} =
             World.Repo.query("UPDATE rooms SET session_overlay = ? WHERE room_id = ?", [
               Jason.encode!(bad),
               room_id
             ])

    assert {:error, :invalid_revision} = Rooms.composed_scene(room_id)

    assert {:error, :invalid_revision} =
             Rooms.append_overlay_entry(room_id, %{"blake3" => @blake3}, 1, 0)
  end

  test "compose_room_scene fail-closed on non-JSON overlay blob" do
    # In-memory Room: Ash Map load cannot surface raw blobs; exercise require_overlay decode.
    assert {:ok, room_id, _, _} = Rooms.create("ov-blob", @owner)
    assert {:ok, room} = Rooms.get(room_id)

    blob_room = %{room | session_overlay: "not-json"}
    assert {:error, :invalid_revision} = Rooms.compose_room_scene(blob_room)

    array_room = %{room | session_overlay: "[1,2]"}
    assert {:error, :invalid_revision} = Rooms.compose_room_scene(array_room)
  end

  test "stride clear miss with corrupt overlay is conflict not invalid_revision" do
    # SQL CAST of non-integer → 0 (has headroom); state race must stay :conflict.
    assert {:ok, room_id, _, _} = Rooms.create("ov-miss-class", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)

    bad = %{"revision" => "x", "entries" => []}

    assert {:ok, _} =
             World.Repo.query("UPDATE rooms SET session_overlay = ? WHERE room_id = ?", [
               Jason.encode!(bad),
               room_id
             ])

    # Active lease still valid → takeover WHERE fails for state, not stride.
    assert {:error, :conflict} = Rooms.takeover(room_id, @owner)
  end

  test "expire_due_host_leases counts stride-blocked due rooms as err" do
    assert {:ok, room_id, _, _} = Rooms.create("ov-sweep-block", @owner)
    assert {:ok, _} = Rooms.activate_as_host(room_id, @owner)

    ceiling = %{"revision" => 999_999, "entries" => []}
    past = DateTime.utc_now() |> DateTime.add(-60, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             World.Repo.query(
               "UPDATE rooms SET session_overlay = ?, host_lease_expires_at = ? WHERE room_id = ?",
               [Jason.encode!(ceiling), past, room_id]
             )

    assert {0, 1} = Rooms.expire_due_host_leases()
    assert {:ok, %{state: "active"}} = Rooms.get(room_id)
  end
end
