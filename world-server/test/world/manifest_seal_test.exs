defmodule World.ManifestSealTest do
  use ExUnit.Case, async: false

  alias World.ManifestSeal
  alias World.Token.Keys

  @blake3 String.duplicate("ab", 32)
  @sub "11111111-1111-1111-1111-111111111111"

  test "generate roundtrip claims" do
    assert {:ok, token, ttl} = ManifestSeal.generate("local", 1, @blake3, @sub)
    assert ttl > 0
    assert is_binary(token)

    signer = Keys.signer()
    assert {:ok, claims} = Joken.verify(token, signer)
    assert claims["typ"] == ManifestSeal.typ()
    assert claims["aud"] == ManifestSeal.audience()
    assert claims["iss"] == "alchemy-world"
    assert claims["room_id"] == "local"
    assert claims["revision"] == 1
    assert claims["manifest_blake3"] == @blake3
    assert claims["sub"] == @sub
  end

  test "generate rejects invalid room_id and blake3" do
    assert {:error, :invalid_room_id} = ManifestSeal.generate("", 1, @blake3, @sub)
    assert {:error, :invalid_room_id} = ManifestSeal.generate("a/b", 1, @blake3, @sub)
    assert {:error, :invalid_manifest_blake3} = ManifestSeal.generate("local", 1, "short", @sub)
  end

  test "parse_revision and parse_blake3" do
    assert {:ok, 7} = ManifestSeal.parse_revision(7)
    assert {:ok, 7} = ManifestSeal.parse_revision("7")
    assert {:error, :invalid_revision} = ManifestSeal.parse_revision(-1)
    assert {:error, :invalid_revision} = ManifestSeal.parse_revision("x")

    assert {:ok, @blake3} = ManifestSeal.parse_blake3(String.upcase(@blake3))
    assert {:error, :invalid_manifest_blake3} = ManifestSeal.parse_blake3("zz")
  end
end
