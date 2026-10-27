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

    roots = Enum.filter(scene["entries"], &(&1["entity_id"] in 3..6))
    assert length(roots) == 4
  end
end
