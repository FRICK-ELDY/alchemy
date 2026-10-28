defmodule World.CanonicalManifestTest do
  use World.DataCase, async: false

  alias World.CanonicalManifest
  alias World.HostAttestation
  alias World.Rooms

  @sub "22222222-2222-2222-2222-222222222222"
  @host_pub :crypto.strong_rand_bytes(32)

  test "canonical_blake3 is stable for demo scene + attestation" do
    assert {:ok, "canon", _vis} = Rooms.create("canon", @sub)

    assert {:ok, attestation, _ttl} =
             HostAttestation.generate("canon", @host_pub, @sub)

    assert {:ok, 1, blake3_a} = CanonicalManifest.canonical_blake3("canon", attestation)
    assert {:ok, 1, blake3_b} = CanonicalManifest.canonical_blake3("canon", attestation)
    assert blake3_a == blake3_b
    assert byte_size(blake3_a) == 64
  end

  test "verify rejects attestation for wrong room" do
    assert {:ok, "room-a", _vis} = Rooms.create("room-a", @sub)
    assert {:ok, "room-b", _vis} = Rooms.create("room-b", @sub)

    assert {:ok, attestation, _} = HostAttestation.generate("room-a", @host_pub, @sub)

    assert {:error, :attestation_room_mismatch} =
             CanonicalManifest.canonical_blake3("room-b", attestation)
  end

  test "protocol_pin config matches alchemy-protocol PROTOCOL_PIN (x0039)" do
    # Keep in lockstep with protocol/src/lib.rs `PROTOCOL_PIN` — drift → seal mismatch.
    assert Application.fetch_env!(:world, :protocol_pin) ==
             "alchemy-protocol/0.15.0-x002e"
  end
end
