defmodule World.AllocationTokenTest do
  use ExUnit.Case, async: false

  alias World.AllocationToken

  test "generate embeds room_jti distinct from alloc jti" do
    room_jti = "11111111-1111-4111-8111-111111111111"

    assert {:ok, token, alloc_jti, ttl} =
             AllocationToken.generate(
               "local",
               "guest",
               "anon",
               "wire-a",
               "alchemy-official",
               room_jti
             )

    assert ttl > 0
    assert alloc_jti != room_jti
    assert {:ok, claims} = AllocationToken.verify(token)
    assert claims["room_jti"] == room_jti
    assert claims["jti"] == alloc_jti
    assert claims["aud"] == "alchemy-relay"
  end

  test "rejects empty or slash room_jti" do
    assert {:error, :invalid_args} =
             AllocationToken.generate("local", "guest", "anon", "wire-a", "alchemy-official", "")

    assert {:error, :invalid_args} =
             AllocationToken.generate(
               "local",
               "guest",
               "anon",
               "wire-a",
               "alchemy-official",
               "bad/jti"
             )
  end
end
