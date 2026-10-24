defmodule World.TokenTest do
  use ExUnit.Case, async: false

  alias World.Token

  test "generate and verify room token roundtrip" do
    assert {:ok, token, ttl} = Token.generate("local", "host", "anon")
    assert ttl > 0
    assert {:ok, claims} = Token.verify(token)
    assert claims["room_id"] == "local"
    assert claims["role"] == "host"
    assert claims["sub"] == "anon"
    assert claims["iss"] == "alchemy-world"
    assert claims["aud"] == "alchemy-room"
  end

  test "rejects invalid role" do
    assert {:error, :invalid_role} = Token.generate("local", "admin", "anon")
  end
end
