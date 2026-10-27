defmodule World.HostAttestationTest do
  use ExUnit.Case, async: false

  alias World.HostAttestation

  @host_pub :crypto.strong_rand_bytes(32)
  @sub "11111111-1111-1111-1111-111111111111"

  test "generate roundtrip claims" do
    assert {:ok, token, ttl} = HostAttestation.generate("local", @host_pub, @sub)
    assert ttl > 0
    assert is_binary(token)

    signer = World.Token.Keys.signer()
    assert {:ok, claims} = Joken.verify(token, signer)
    assert claims["typ"] == HostAttestation.typ()
    assert claims["aud"] == HostAttestation.audience()
    assert claims["iss"] == "alchemy-world"
    assert claims["room_id"] == "local"
    assert claims["sub"] == @sub
    assert claims["host_pub"] == Base.url_encode64(@host_pub, padding: false)
  end

  test "generate rejects invalid room_id" do
    assert {:error, :invalid_room_id} = HostAttestation.generate("", @host_pub, @sub)
    assert {:error, :invalid_room_id} = HostAttestation.generate("a/b", @host_pub, @sub)

    too_long = String.duplicate("r", 65)
    assert {:error, :invalid_room_id} = HostAttestation.generate(too_long, @host_pub, @sub)
  end

  test "generate rejects wrong host_pub length" do
    assert {:error, :invalid_host_pub} = HostAttestation.generate("local", <<1, 2, 3>>, @sub)
  end

  test "decode_host_pub accepts 32-byte base64url" do
    encoded = Base.url_encode64(@host_pub, padding: false)
    assert {:ok, @host_pub} = HostAttestation.decode_host_pub(encoded)
  end

  test "decode_host_pub rejects bad input" do
    assert {:error, :invalid_host_pub} = HostAttestation.decode_host_pub("AA")
    assert {:error, :invalid_host_pub} = HostAttestation.decode_host_pub("not-base64!!!")
    assert {:error, :invalid_host_pub} = HostAttestation.decode_host_pub(nil)
  end
end
