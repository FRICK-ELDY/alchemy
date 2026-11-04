defmodule Assets.DriveTest do
  use Assets.DataCase, async: false

  alias Assets.Blobs
  alias Assets.Drive
  alias Assets.Prefab.Payload

  @sub_a "55555555-5555-5555-5555-555555555555"
  @sub_b "66666666-6666-6666-6666-666666666666"
  @blake3_a String.duplicate("a", 64)
  @blake3_b String.duplicate("b", 64)

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    :ok
  end

  test "folder CRUD is owner-only" do
    assert {:ok, folder} = Drive.create_folder(@sub_a, "Root")
    assert folder.kind == "folder"
    assert Drive.get_node(@sub_b, folder.node_id) == nil
    assert Drive.list_children(@sub_a, nil) == [folder]
    assert Drive.list_children(@sub_b, nil) == []
    assert :ok = Drive.delete_node(@sub_a, folder.node_id)
  end

  test "blob leaf requires grant; other owner forbidden parent" do
    body = "drive-blob-leaf"
    assert {:ok, obj, _} = Blobs.upload(@sub_a, body, "text/plain")
    assert {:ok, folder} = Drive.create_folder(@sub_a, "Inv")
    assert {:ok, leaf} = Drive.create_blob(@sub_a, "item", obj.blake3, folder.node_id)
    assert leaf.kind == "blob"
    assert leaf.blake3 == obj.blake3

    assert {:error, :no_grant} = Drive.create_blob(@sub_a, "nope", @blake3_b, folder.node_id)
    assert {:error, :forbidden} = Drive.create_blob(@sub_b, "steal", obj.blake3, folder.node_id)
  end

  test "cycle depth exceeded when parent chain is too deep" do
    assert {:ok, n0} = Drive.create_folder(@sub_a, "n0")

    # 32 nested folders under n0 → walk from leaf needs 33 hops (> @max_cycle_depth 32)
    last =
      Enum.reduce(1..32, n0.node_id, fn i, pid ->
        assert {:ok, n} = Drive.create_folder(@sub_a, "n#{i}", pid)
        n.node_id
      end)

    assert {:error, :cycle_depth_exceeded} = Drive.create_folder(@sub_a, "too-deep", last)
  end

  test "resolve_for_take_out and save_prefab round-trip" do
    mesh = "mesh-bytes"
    assert {:ok, mesh_obj, _} = Blobs.upload(@sub_a, mesh, "text/plain")

    {:ok, payload} =
      Payload.new(1, [
        %{
          local_id: "root",
          blake3: mesh_obj.blake3,
          trs: %{
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0]
          }
        }
      ])

    assert {:ok, node} = Drive.save_prefab(@sub_a, "MyPrefab", payload)
    assert node.kind == "prefab"
    assert {:ok, %{blobs: [%{blake3: blake3}]}} = Drive.resolve_for_take_out(@sub_a, node.node_id)
    assert blake3 == node.blake3

    assert {:ok, json} = Assets.CAS.read(node.blake3)
    assert {:ok, reloaded} = Payload.from_json(json)
    assert reloaded.schema_version == 1
    assert length(reloaded.children) == 1
  end

  test "prefab payload rejects non-numeric TRS" do
    assert {:error, _} =
             Payload.new(1, [
               %{
                 local_id: "bad",
                 blake3: @blake3_a,
                 trs: %{
                   translation: [0.0, "NaN", 0.0],
                   rotation: [0.0, 0.0, 0.0, 1.0],
                   scale: [1.0, 1.0, 1.0]
                 }
               }
             ])
  end
end
