defmodule World.DemoSceneVectorTest do
  use ExUnit.Case, async: true

  alias World.CanonicalManifest
  alias World.DemoScene
  alias World.DemoSceneVector
  alias World.ManifestSealHash

  test "DemoScene seal matches protocol shared vector (x0079)" do
    scene = DemoScene.build()
    assert scene["revision"] == DemoSceneVector.revision()

    payload =
      CanonicalManifest.build_payload(
        DemoSceneVector.room_id(),
        DemoSceneVector.revision(),
        DemoSceneVector.host_generation(),
        scene,
        DemoSceneVector.host_pub(),
        DemoSceneVector.host_attestation()
      )

    assert payload["room_id"] == DemoSceneVector.room_id()
    assert payload["host_pub_hex"] == DemoSceneVector.host_pub_hex()
    assert payload["protocol_pin"] == Application.fetch_env!(:world, :protocol_pin)

    assert {:ok, blake3} = ManifestSealHash.blake3_hex(payload)
    assert blake3 == DemoSceneVector.blake3()
  end
end
