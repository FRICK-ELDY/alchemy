defmodule World.CanonicalManifestTest do
  use World.DataCase, async: false

  alias World.CanonicalManifest
  alias World.HostAttestation
  alias World.Rooms

  @sub "22222222-2222-2222-2222-222222222222"
  @host_pub :crypto.strong_rand_bytes(32)

  test "canonical_blake3 is stable for demo scene + attestation" do
    assert {:ok, "canon", _vis, _cap} = Rooms.create("canon", @sub)
    assert {:ok, gen} = Rooms.claim_host_generation("canon", @host_pub)

    assert {:ok, attestation, _ttl} =
             HostAttestation.generate("canon", @host_pub, @sub, gen)

    # x009e: host-facing revision is pack(canonical=1, overlay=0).
    assert {:ok, packed} = Rooms.pack_revision(1, 0)

    assert {:ok, ^packed, ^gen, blake3_a} =
             CanonicalManifest.canonical_blake3("canon", attestation)

    assert {:ok, ^packed, ^gen, blake3_b} =
             CanonicalManifest.canonical_blake3("canon", attestation)

    assert blake3_a == blake3_b
    assert byte_size(blake3_a) == 64
  end

  test "rejects TTL-valid attestation after host_pub handoff (x003a)" do
    assert {:ok, "stale-seal", _vis, _cap} = Rooms.create("stale-seal", @sub)
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
    assert {:ok, "room-a", _vis, _cap} = Rooms.create("room-a", @sub)
    assert {:ok, "room-b", _vis, _cap} = Rooms.create("room-b", @sub)
    assert {:ok, gen} = Rooms.claim_host_generation("room-a", @host_pub)

    assert {:ok, attestation, _} = HostAttestation.generate("room-a", @host_pub, @sub, gen)

    assert {:error, :attestation_room_mismatch} =
             CanonicalManifest.canonical_blake3("room-b", attestation)
  end

  test "protocol_pin config matches alchemy-protocol PROTOCOL_PIN (x0039/x003a)" do
    # Keep in lockstep with protocol/src/lib.rs `PROTOCOL_PIN` — drift → seal mismatch.
    # x003b decode limits do not bump the pin (wire-compatible).
    assert Application.fetch_env!(:world, :protocol_pin) ==
             "alchemy-protocol/0.24.0-x00b8"
  end

  test "seal payload includes parent_entity_id and parent diffs change blake3 (x00b8)" do
    # CI builds `manifest-seal-hash` before mix test (world-server-ci.yml).
    # Local: rebuild that binary if this assertion flakes after a proto change.
    # Hash-diff coverage also lives in alchemy-protocol `parent_entity_id_affects_seal_blake3`.
    scene_a = %{
      "revision" => 1,
      "locomotion" => %{"mode" => "noclip", "move_speed" => 4.0},
      "entries" => [
        %{"entity_id" => 1, "blake3" => "", "parent_entity_id" => 0},
        %{"entity_id" => 2, "blake3" => "", "parent_entity_id" => 1}
      ]
    }

    scene_b = put_in(scene_a, ["entries", Access.at(1), "parent_entity_id"], 0)

    payload_a =
      CanonicalManifest.build_payload(
        "parent-seal",
        1,
        1,
        scene_a,
        <<0::256>>,
        "attestation"
      )

    payload_b =
      CanonicalManifest.build_payload(
        "parent-seal",
        1,
        1,
        scene_b,
        <<0::256>>,
        "attestation"
      )

    assert Enum.at(payload_a["entries"], 1)["parent_entity_id"] == 1
    assert Enum.at(payload_b["entries"], 1)["parent_entity_id"] == 0

    assert {:ok, blake_a} = World.ManifestSealHash.blake3_hex(payload_a)
    assert {:ok, blake_b} = World.ManifestSealHash.blake3_hex(payload_b)
    assert blake_a != blake_b
  end
end
