defmodule World.DemoSceneTest do
  use ExUnit.Case, async: true

  alias World.DemoScene

  test "build returns coin, user_roots, and grabbable boxes" do
    scene = DemoScene.build()
    assert scene["revision"] == 1
    assert scene["locomotion"]["mode"] == "noclip"
    assert length(scene["entries"]) == 1 + 4 + 9

    coin = Enum.find(scene["entries"], &(&1["entity_id"] == 1))
    assert is_binary(coin["blake3"])
    assert byte_size(coin["blake3"]) == 64

    boxes = Enum.filter(scene["entries"], &(&1["half_extent"] > 0))
    assert length(boxes) == 9
    assert Enum.all?(boxes, &(&1["flags"] == 1))

    assert Enum.all?(scene["entries"], fn e ->
             Map.has_key?(e, "qx") and Map.has_key?(e, "sx")
           end)

    center = Enum.find(boxes, &(&1["entity_id"] == 14))
    assert_in_delta center["qy"], 0.3826834323650898, 1.0e-9
    assert_in_delta center["qw"], 0.9238795325112867, 1.0e-9
    assert_in_delta center["sx"], 1.5, 1.0e-9
    assert_in_delta center["sy"], 1.0, 1.0e-9

    roots = Enum.filter(scene["entries"], &(&1["entity_id"] in 3..6))
    assert length(roots) == 4
  end
end
