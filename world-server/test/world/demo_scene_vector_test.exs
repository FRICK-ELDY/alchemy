defmodule World.DemoSceneVectorTest do
  use ExUnit.Case, async: true

  alias World.DemoScene
  alias World.DemoSceneVector
  alias World.ManifestSealHash

  test "DemoScene seal matches protocol shared vector (x0079)" do
    assert DemoScene.build()["revision"] == DemoSceneVector.revision()

    payload = DemoSceneVector.payload()
    assert payload["room_id"] == DemoSceneVector.room_id()
    assert payload["host_pub_hex"] == DemoSceneVector.host_pub_hex()
    assert payload["protocol_pin"] == Application.fetch_env!(:world, :protocol_pin)
    assert payload["locomotion"]["mode"] == "noclip"
    assert payload["locomotion"]["move_speed"] == 4.0

    assert {:ok, blake3} = ManifestSealHash.blake3_hex(payload)
    assert blake3 == DemoSceneVector.blake3()
  end
end
