defmodule Assets.QuarantineTest do
  use ExUnit.Case, async: true

  alias Assets.Quarantine
  alias Assets.Quarantine.HeaderCheckWorker

  @blake3 String.duplicate("a", 64)

  test "HeaderCheckWorker accepts valid GLB magic for gltf-binary MIME" do
    data = "glTF" <> <<2, 0, 0, 0>> <> :crypto.strong_rand_bytes(8)
    assert :ok = HeaderCheckWorker.inspect_blob(@blake3, data, "model/gltf-binary")
  end

  test "HeaderCheckWorker quarantines invalid GLB header" do
    assert {:quarantine, :invalid_glb_header} =
             HeaderCheckWorker.inspect_blob(@blake3, "not-a-glb", "model/gltf-binary")
  end

  test "HeaderCheckWorker strips Content-Type parameters before MIME match" do
    assert {:quarantine, :invalid_glb_header} =
             HeaderCheckWorker.inspect_blob(
               @blake3,
               "not-a-glb",
               "model/gltf-binary; charset=binary"
             )
  end

  test "HeaderCheckWorker ignores non-glb MIME including octet-stream" do
    assert :ok =
             HeaderCheckWorker.inspect_blob(@blake3, "not-a-glb", "application/octet-stream")
  end

  test "Quarantine façade uses configured HeaderCheckWorker by default" do
    assert {:quarantine, :invalid_glb_header} =
             Quarantine.inspect_blob(@blake3, "x", "model/gltf-binary")
  end

  test "public_reason allowlists OSS codes and collapses unknown payloads" do
    assert Quarantine.public_reason(:invalid_glb_header) == "invalid_glb_header"
    assert Quarantine.public_reason({:secret_threshold, 0.87}) == "rejected"
  end
end
