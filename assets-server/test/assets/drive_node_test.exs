defmodule Assets.DriveNodeTest do
  @moduledoc """
  Tests for x005c (DriveNode folder/blob) and x005d (kind=prefab).
  """
  use Assets.DataCase, async: false

  alias Assets.Blobs
  alias Assets.Drive
  alias Assets.Drive.Node

  @sub_a "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
  @sub_b "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    :ok
  end

  # ---------------------------------------------------------------------------
  # x005c — folder
  # ---------------------------------------------------------------------------

  test "create_folder returns node with kind=folder" do
    assert {:ok, %Node{kind: "folder", owner_sub: @sub_a, parent_id: nil}} =
             Drive.create_folder(@sub_a, "My Folder")
  end

  test "create_folder with parent nests correctly" do
    {:ok, parent} = Drive.create_folder(@sub_a, "Parent")
    assert {:ok, %Node{parent_id: pid}} = Drive.create_folder(@sub_a, "Child", parent.node_id)
    assert pid == parent.node_id
  end

  test "create_folder parent owned by other user is forbidden" do
    {:ok, other_folder} = Drive.create_folder(@sub_b, "Other")
    assert {:error, :forbidden} = Drive.create_folder(@sub_a, "Hijack", other_folder.node_id)
  end

  test "create_folder with non-folder parent is rejected" do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "blob-data", "application/octet-stream")

    {:ok, blob_node} = Drive.create_blob(@sub_a, "my-blob", cas_obj.blake3)
    assert {:error, :parent_not_folder} = Drive.create_folder(@sub_a, "Nested", blob_node.node_id)
  end

  test "list_children returns only owner root nodes when parent_id is nil" do
    {:ok, f1} = Drive.create_folder(@sub_a, "F1")
    {:ok, _f2} = Drive.create_folder(@sub_a, "F2")
    {:ok, _b_other} = Drive.create_folder(@sub_b, "Other")

    children = Drive.list_children(@sub_a)
    node_ids = Enum.map(children, & &1.node_id)
    assert f1.node_id in node_ids
    assert length(children) == 2
  end

  test "list_children returns folder contents" do
    {:ok, folder} = Drive.create_folder(@sub_a, "Folder")
    {:ok, child} = Drive.create_folder(@sub_a, "Child", folder.node_id)
    {:ok, _other_root} = Drive.create_folder(@sub_a, "OtherRoot")

    children = Drive.list_children(@sub_a, folder.node_id)
    assert length(children) == 1
    assert hd(children).node_id == child.node_id
  end

  test "get_node returns nil for wrong owner" do
    {:ok, node} = Drive.create_folder(@sub_a, "Private")
    assert nil == Drive.get_node(@sub_b, node.node_id)
  end

  test "get_node returns node for owner" do
    {:ok, node} = Drive.create_folder(@sub_a, "Mine")
    assert %Node{node_id: id} = Drive.get_node(@sub_a, node.node_id)
    assert id == node.node_id
  end

  test "delete_node removes a leaf folder" do
    {:ok, node} = Drive.create_folder(@sub_a, "Empty")
    assert :ok = Drive.delete_node(@sub_a, node.node_id)
    assert nil == Drive.get_node(@sub_a, node.node_id)
  end

  test "delete_node refuses non-empty folder" do
    {:ok, parent} = Drive.create_folder(@sub_a, "Parent")
    {:ok, _child} = Drive.create_folder(@sub_a, "Child", parent.node_id)
    assert {:error, :not_empty} = Drive.delete_node(@sub_a, parent.node_id)
  end

  test "delete_node returns not_found for wrong owner" do
    {:ok, node} = Drive.create_folder(@sub_a, "Mine")
    assert {:error, :not_found} = Drive.delete_node(@sub_b, node.node_id)
  end

  # ---------------------------------------------------------------------------
  # x005c — blob leaf
  # ---------------------------------------------------------------------------

  test "create_blob requires BlobGrant — no grant returns :no_grant" do
    fake_blake3 = String.duplicate("a", 64)
    assert {:error, :no_grant} = Drive.create_blob(@sub_a, "no-grant-blob", fake_blake3)
  end

  test "create_blob succeeds after upload (BlobGrant created by upload)" do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "hello-blob", "text/plain")

    assert {:ok, %Node{kind: "blob", blake3: b3}} =
             Drive.create_blob(@sub_a, "blob-leaf", cas_obj.blake3)

    assert b3 == cas_obj.blake3
  end

  test "create_blob with other user's blake3 denied" do
    {:ok, cas_obj, _} = Blobs.upload(@sub_b, "their-data", "text/plain")
    # sub_a has no BlobGrant for sub_b's blake3
    assert {:error, :no_grant} = Drive.create_blob(@sub_a, "stolen", cas_obj.blake3)
  end

  # ---------------------------------------------------------------------------
  # x005d — prefab kind
  # ---------------------------------------------------------------------------

  test "create_prefab_node requires BlobGrant" do
    fake_blake3 = String.duplicate("f", 64)
    assert {:error, :no_grant} = Drive.create_prefab_node(@sub_a, "prefab", fake_blake3)
  end

  test "create_prefab_node succeeds with grant" do
    {:ok, cas_obj, _} =
      Blobs.upload(@sub_a, ~s({"schema_version":1,"children":[]}), "application/json",
        access: :grant_required
      )

    assert {:ok, %Node{kind: "prefab", blake3: b3}} =
             Drive.create_prefab_node(@sub_a, "my-prefab", cas_obj.blake3)

    assert b3 == cas_obj.blake3
  end

  test "folder / blob / prefab coexist under same parent" do
    {:ok, parent} = Drive.create_folder(@sub_a, "Container")
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "raw-mesh-bytes", "application/octet-stream")

    {:ok, _prefab_cas, _} =
      Blobs.upload(@sub_a, ~s({"schema_version":1,"children":[]}), "application/json",
        access: :grant_required
      )

    {:ok, blob_node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3, parent.node_id)

    {:ok, prefab_cas, _} =
      Blobs.upload(@sub_a, ~s({"schema_version":1,"children":[]}), "application/json",
        access: :grant_required
      )

    {:ok, prefab_node} =
      Drive.create_prefab_node(@sub_a, "my-prefab", prefab_cas.blake3, parent.node_id)

    {:ok, sub_folder} = Drive.create_folder(@sub_a, "sub", parent.node_id)

    children = Drive.list_children(@sub_a, parent.node_id)
    kinds = Enum.map(children, & &1.kind)
    assert "blob" in kinds
    assert "prefab" in kinds
    assert "folder" in kinds
    assert length(children) == 3

    _ = blob_node
    _ = prefab_node
    _ = sub_folder
  end

  test "prefab without grant cannot be created via create_prefab_node" do
    fake = String.duplicate("0", 64)
    assert {:error, :no_grant} = Drive.create_prefab_node(@sub_a, "bad", fake)
  end

  test "unknown kind is rejected at schema level" do
    changeset =
      Node.changeset(%Node{}, %{
        node_id: "00000000-0000-0000-0000-000000000001",
        owner_sub: @sub_a,
        kind: "unknown_kind",
        name: "X"
      })

    refute changeset.valid?
  end
end
