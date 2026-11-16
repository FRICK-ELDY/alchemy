defmodule Assets.VcsObjectTest do
  @moduledoc "Port pool round-trips for vcs-object (x00c9)."
  use ExUnit.Case, async: false

  alias Assets.VcsObject

  @zero String.duplicate("0", 64)

  test "encode_tree + verify round-trip" do
    assert {:ok, %{bytes: bytes, hash_hex: hash}} =
             VcsObject.encode_tree(%{
               "children" => [%{"local_id" => "a", "blake3" => @zero}]
             })

    assert byte_size(hash) == 64
    assert {:ok, summary} = VcsObject.verify(bytes)
    assert summary["ok"] == true
    assert summary["kind"] == "tree"
    assert summary["hash_hex"] == hash
    assert summary["child_blake3s_hex"] == [@zero]
  end

  test "encode_commit + verify round-trip" do
    assert {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} =
             VcsObject.encode_tree(%{
               "children" => [%{"local_id" => "root", "blake3" => @zero}]
             })

    assert {:ok, _} = VcsObject.verify(tree_bytes)

    assert {:ok, %{bytes: commit_bytes, hash_hex: commit_hash}} =
             VcsObject.encode_commit(%{
               "tree" => tree_hash,
               "parents" => [],
               "generation" => 1,
               "author_sub" => "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
               "created_at_ms" => 1_700_000_000_000,
               "message" => "promote"
             })

    assert {:ok, summary} = VcsObject.verify(commit_bytes)
    assert summary["kind"] == "commit"
    assert summary["hash_hex"] == commit_hash
    assert summary["tree_hex"] == tree_hash
    assert summary["parents_hex"] == []
    assert summary["generation"] == 1
  end

  test "encode_tree rejects invalid blake3" do
    assert {:error, _} =
             VcsObject.encode_tree(%{
               "children" => [%{"local_id" => "a", "blake3" => "not-hex"}]
             })
  end
end
