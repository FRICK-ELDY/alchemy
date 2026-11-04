defmodule Assets.DriveTakeOutTest do
  @moduledoc """
  Tests for x005f (Drive.resolve_for_take_out) and x0060/x0061 (save_prefab + roundtrip).
  """
  use Assets.DataCase, async: false

  alias Assets.Blobs
  alias Assets.Drive
  alias Assets.Prefab.Payload

  @sub_a "cccccccc-cccc-cccc-cccc-cccccccccccc"
  @sub_b "dddddddd-dddd-dddd-dddd-dddddddddddd"

  @blake3 String.duplicate("a", 64)

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    :ok
  end

  # ---------------------------------------------------------------------------
  # x005f — resolve_for_take_out
  # ---------------------------------------------------------------------------

  test "resolve_for_take_out returns node and blobs for blob DriveNode" do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "mesh-data", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    assert {:ok, %{node: n, blobs: blobs}} = Drive.resolve_for_take_out(@sub_a, node.node_id)
    assert n.node_id == node.node_id
    assert [%{blake3: b3}] = blobs
    assert b3 == cas_obj.blake3
  end

  test "resolve_for_take_out returns empty blobs for folder" do
    {:ok, folder} = Drive.create_folder(@sub_a, "Folder")
    assert {:ok, %{blobs: []}} = Drive.resolve_for_take_out(@sub_a, folder.node_id)
  end

  test "resolve_for_take_out returns not_found for wrong owner" do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "private-mesh", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    assert {:error, :not_found} = Drive.resolve_for_take_out(@sub_b, node.node_id)
  end

  test "resolve_for_take_out returns not_found for unknown node" do
    assert {:error, :not_found} =
             Drive.resolve_for_take_out(@sub_a, "00000000-0000-0000-0000-000000000001")
  end

  test "resolve_for_take_out for prefab returns payload blob" do
    {:ok, payload} =
      Payload.new(1, [%{local_id: "m0", blake3: @blake3, trs: nil}])

    {:ok, node} = Drive.save_prefab(@sub_a, "my-prefab", payload)

    assert {:ok, %{node: n, blobs: [%{blake3: b3}]}} =
             Drive.resolve_for_take_out(@sub_a, node.node_id)

    assert n.kind == "prefab"
    assert is_binary(b3) and byte_size(b3) == 64
  end

  # ---------------------------------------------------------------------------
  # x0060 — save_prefab (put-back)
  # ---------------------------------------------------------------------------

  test "save_prefab creates DriveNode(kind=prefab) with validated payload" do
    {:ok, payload} = Payload.new(1, [%{local_id: "c0", blake3: @blake3, trs: nil}])
    assert {:ok, node} = Drive.save_prefab(@sub_a, "weapon-prefab", payload)

    assert node.kind == "prefab"
    assert node.owner_sub == @sub_a
    assert node.name == "weapon-prefab"
    assert byte_size(node.blake3) == 64
  end

  test "save_prefab upserts on same name+owner+parent" do
    {:ok, p1} = Payload.new(1, [%{local_id: "c0", blake3: @blake3, trs: nil}])
    {:ok, node1} = Drive.save_prefab(@sub_a, "shared-name", p1)

    blake3b = String.duplicate("b", 64)
    {:ok, p2} = Payload.new(1, [%{local_id: "c1", blake3: blake3b, trs: nil}])
    {:ok, node2} = Drive.save_prefab(@sub_a, "shared-name", p2)

    # Same node_id (upsert)
    assert node1.node_id == node2.node_id
    # New blake3 (payload updated)
    assert node1.blake3 != node2.blake3
  end

  test "duplicate prefab insert surfaces as changeset unique (not ConstraintError)" do
    alias Assets.Drive.Node

    {:ok, payload} = Payload.new(1, [%{local_id: "c0", blake3: @blake3, trs: nil}])
    {:ok, existing} = Drive.save_prefab(@sub_a, "uniq-prefab", payload)

    other_blake3 = String.duplicate("d", 64)

    cs =
      Node.changeset(%Node{}, %{
        node_id: Ecto.UUID.generate(),
        owner_sub: @sub_a,
        parent_id: nil,
        kind: "prefab",
        name: "uniq-prefab",
        blake3: other_blake3
      })

    # Must be Changeset error (ecto_sqlite3 name mapped), never raise ConstraintError.
    assert {:error, %Ecto.Changeset{errors: errors}} = Repo.insert(cs)

    assert Enum.any?(errors, fn
             {_field, {_msg, opts}} -> opts[:constraint] == :unique
             _ -> false
           end)

    # save_prefab still updates the existing row via find/upsert.
    {:ok, p2} = Payload.new(1, [%{local_id: "c1", blake3: other_blake3, trs: nil}])
    assert {:ok, updated} = Drive.save_prefab(@sub_a, "uniq-prefab", p2)
    assert updated.node_id == existing.node_id
    assert updated.blake3 != existing.blake3
  end

  test "save_prefab for different names creates separate nodes" do
    {:ok, payload} = Payload.new(1, [])
    {:ok, n1} = Drive.save_prefab(@sub_a, "prefab-one", payload)
    {:ok, n2} = Drive.save_prefab(@sub_a, "prefab-two", payload)
    assert n1.node_id != n2.node_id
  end

  test "save_prefab in a parent folder" do
    {:ok, folder} = Drive.create_folder(@sub_a, "MyFolder")
    {:ok, payload} = Payload.new(1, [])
    assert {:ok, node} = Drive.save_prefab(@sub_a, "nested-prefab", payload, folder.node_id)
    assert node.parent_id == folder.node_id
  end

  test "save_prefab rejects parent owned by other user" do
    {:ok, other_folder} = Drive.create_folder(@sub_b, "Other")
    {:ok, payload} = Payload.new(1, [])
    assert {:error, :forbidden} = Drive.save_prefab(@sub_a, "bad", payload, other_folder.node_id)
  end

  # ---------------------------------------------------------------------------
  # x0061 — put-back → reload → take-out roundtrip
  # ---------------------------------------------------------------------------

  test "save_prefab → reload → Payload.from_json roundtrip preserves logical tree (x0061)" do
    children = [
      %{local_id: "mesh_0", blake3: @blake3, trs: nil},
      %{
        local_id: "mesh_1",
        blake3: String.duplicate("c", 64),
        trs: %{
          translation: [1.0, 2.0, 3.0],
          rotation: [0.0, 0.0, 0.0, 1.0],
          scale: [1.0, 1.0, 1.0]
        }
      }
    ]

    {:ok, original_payload} = Payload.new(1, children)
    {:ok, node} = Drive.save_prefab(@sub_a, "roundtrip-prefab", original_payload)

    # Reload node
    reloaded_node = Drive.get_node(@sub_a, node.node_id)
    assert reloaded_node.kind == "prefab"
    assert reloaded_node.blake3 == node.blake3

    # Fetch payload bytes from CAS and decode
    {:ok, json_bytes, _cas_obj} = Assets.Blobs.fetch(reloaded_node.blake3, @sub_a)
    assert {:ok, decoded} = Payload.from_json(json_bytes)

    assert decoded.schema_version == 1
    assert length(decoded.children) == 2
    [c0, c1] = decoded.children
    assert c0.local_id == "mesh_0"
    assert c0.blake3 == @blake3
    assert is_nil(c0.trs)
    assert c1.local_id == "mesh_1"
    assert c1.trs.translation == [1.0, 2.0, 3.0]
  end

  test "no-grant user cannot take out owner's prefab (x0061 — Grant persists across reload)" do
    {:ok, payload} = Payload.new(1, [%{local_id: "c0", blake3: @blake3, trs: nil}])
    {:ok, node} = Drive.save_prefab(@sub_a, "private", payload)

    # sub_b sees not_found because Drive.get_node is owner-only
    assert {:error, :not_found} = Drive.resolve_for_take_out(@sub_b, node.node_id)
  end
end
