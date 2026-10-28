defmodule World.CanonicalManifestTest do
  use World.DataCase, async: false

  alias World.CanonicalManifest
  alias World.HostAttestation
  alias World.Rooms

  @sub "22222222-2222-2222-2222-222222222222"
  @host_pub :crypto.strong_rand_bytes(32)

  test "canonical_blake3 is stable for demo scene + attestation" do
    assert {:ok, "canon", _vis} = Rooms.create("canon", @sub)
    assert {:ok, gen} = Rooms.claim_host_generation("canon", @host_pub)

    assert {:ok, attestation, _ttl} =
             HostAttestation.generate("canon", @host_pub, @sub, gen)

    assert {:ok, 1, ^gen, blake3_a} = CanonicalManifest.canonical_blake3("canon", attestation)
    assert {:ok, 1, ^gen, blake3_b} = CanonicalManifest.canonical_blake3("canon", attestation)
    assert blake3_a == blake3_b
    assert byte_size(blake3_a) == 64
  end

  test "rejects TTL-valid attestation after host_pub handoff (x003a)" do
    assert {:ok, "stale-seal", _vis} = Rooms.create("stale-seal", @sub)
    assert {:ok, gen1} = Rooms.claim_host_generation("stale-seal", @host_pub)

    assert {:ok, old_attestation, _} =
             HostAttestation.generate("stale-seal", @host_pub, @sub, gen1)

    other = :crypto.strong_rand_bytes(32)
    assert {:ok, gen2} = Rooms.claim_host_generation("stale-seal", other)
    assert gen2 == gen1 + 1

    assert {:error, :stale_host_attestation} =
             CanonicalManifest.canonical_blake3("stale-seal", old_attestation)
  end

  test "verify rejects attestation for wrong room" do
    assert {:ok, "room-a", _vis} = Rooms.create("room-a", @sub)
    assert {:ok, "room-b", _vis} = Rooms.create("room-b", @sub)
    assert {:ok, gen} = Rooms.claim_host_generation("room-a", @host_pub)

    assert {:ok, attestation, _} = HostAttestation.generate("room-a", @host_pub, @sub, gen)

    assert {:error, :attestation_room_mismatch} =
             CanonicalManifest.canonical_blake3("room-b", attestation)
  end

  test "protocol_pin config matches alchemy-protocol PROTOCOL_PIN (x0039/x003a)" do
    # Keep in lockstep with protocol/src/lib.rs `PROTOCOL_PIN` — drift → seal mismatch.
    # x003b decode limits do not bump the pin (wire-compatible).
    assert Application.fetch_env!(:world, :protocol_pin) ==
             "alchemy-protocol/0.16.0-x003a"
  end
end
