defmodule World.TokenTest do
  use ExUnit.Case, async: false

  alias World.Token

  defp sample_session_pub do
    :crypto.strong_rand_bytes(32)
  end

  test "generate and verify room token roundtrip embeds cnf.spk" do
    session_pub = sample_session_pub()
    assert {:ok, token, ttl} = Token.generate("local", "host", "anon", session_pub)
    assert ttl > 0
    assert {:ok, claims} = Token.verify(token)
    assert claims["room_id"] == "local"
    assert claims["role"] == "host"
    assert claims["sub"] == "anon"
    assert claims["iss"] == "alchemy-world"
    assert claims["aud"] == "alchemy-room"
    assert claims["cnf"]["spk"] == Base.url_encode64(session_pub, padding: false)
  end

  test "rejects invalid role" do
    assert {:error, :invalid_role} =
             Token.generate("local", "admin", "anon", sample_session_pub())
  end

  test "rejects non-32-byte session_pub" do
    assert {:error, :invalid_args} = Token.generate("local", "host", "anon", <<1, 2, 3>>)
  end

  test "decode_session_pub accepts 32-byte base64url" do
    raw = sample_session_pub()
    encoded = Base.url_encode64(raw, padding: false)
    assert {:ok, ^raw} = Token.decode_session_pub(encoded)
    assert {:error, :invalid_session_pub} = Token.decode_session_pub("not-base64!!!")
    assert {:error, :invalid_session_pub} = Token.decode_session_pub("")
  end

  test "generated token verifies with cnf; decode rejects wrong length" do
    session_pub = sample_session_pub()
    assert {:ok, token, _ttl} = Token.generate("local", "guest", "anon", session_pub)
    assert {:ok, claims} = Token.verify(token)
    assert claims["cnf"]["spk"] == Base.url_encode64(session_pub, padding: false)

    assert {:error, :invalid_session_pub} = Token.decode_session_pub("")
    assert {:error, :invalid_session_pub} = Token.decode_session_pub("AAAA")
    short = Base.url_encode64(<<1, 2, 3>>, padding: false)
    assert {:error, :invalid_session_pub} = Token.decode_session_pub(short)
  end
end
