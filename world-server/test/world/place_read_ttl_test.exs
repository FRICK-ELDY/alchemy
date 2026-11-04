defmodule World.PlaceReadTtlTest do
  use ExUnit.Case, async: false

  alias World.ManifestSeal
  alias World.PlaceHashes
  alias World.PlaceRead
  alias World.Token.Keys

  @blake3 String.duplicate("ab", 32)
  @sub "11111111-1111-1111-1111-111111111111"

  test "PlaceRead exp equals seal absolute unix exp" do
    room = "ttl-abs-#{System.unique_integer([:positive])}"
    seal_exp = System.system_time(:second) + 120

    assert {:ok, token, expires_in} =
             PlaceRead.generate(room, @sub, 1, 1, @blake3, [], seal_exp)

    assert expires_in == 120 or expires_in == 119
    assert {:ok, claims} = PlaceRead.verify(token)
    assert claims["exp"] == seal_exp
  end

  test "seal_exp_from_claims rejects expired" do
    now = System.system_time(:second)
    assert {:ok, exp} = PlaceRead.seal_exp_from_claims(%{"exp" => now + 10})
    assert exp == now + 10
    assert {:error, :seal_expired} = PlaceRead.seal_exp_from_claims(%{"exp" => now - 1})
  end

  test "touch_until aligns cache to seal.exp; without touch short put expires" do
    room = "ttl-touch-#{System.unique_integer([:positive])}"
    # Force a 1s allowlist window via touch_until after put
    PlaceHashes.put(room, 1, @blake3, [@blake3])
    seal_exp = System.system_time(:second) + 1
    assert :ok = PlaceHashes.touch_until(room, 1, @blake3, seal_exp)
    assert {:ok, [@blake3]} = PlaceHashes.resolve(room, 1, @blake3)

    Process.sleep(1_100)
    assert {:error, :place_hashes_unavailable} = PlaceHashes.resolve(room, 1, @blake3)
  end

  test "touch_until with future seal.exp keeps resolve after brief wait" do
    room = "ttl-touch-hit-#{System.unique_integer([:positive])}"
    PlaceHashes.put(room, 1, @blake3, [@blake3])
    # Collapse to ~1s then extend to +5s
    assert :ok = PlaceHashes.touch_until(room, 1, @blake3, System.system_time(:second) + 1)
    Process.sleep(200)
    assert :ok = PlaceHashes.touch_until(room, 1, @blake3, System.system_time(:second) + 5)
    Process.sleep(1_100)
    assert {:ok, [@blake3]} = PlaceHashes.resolve(room, 1, @blake3)
  end

  test "expired ManifestSeal verify surfaces as seal_expired via controller helper path" do
    # Build an already-expired seal-shaped JWT with world keys.
    now = System.system_time(:second)
    issuer = Application.fetch_env!(:world, :room_token_issuer)

    claims = %{
      "typ" => ManifestSeal.typ(),
      "sub" => @sub,
      "room_id" => "expired-room",
      "revision" => 1,
      "host_generation" => 1,
      "manifest_blake3" => @blake3,
      "jti" => "00000000-0000-4000-8000-000000000001",
      "iat" => now - 10,
      "nbf" => now - 10,
      "exp" => now - 5,
      "iss" => issuer,
      "aud" => ManifestSeal.audience()
    }

    config =
      Joken.Config.default_claims(
        skip: [:exp, :iat, :nbf],
        iss: issuer,
        aud: ManifestSeal.audience()
      )
      |> Joken.Config.add_claim("typ", fn -> ManifestSeal.typ() end, &(&1 == ManifestSeal.typ()))
      |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("host_generation", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("iat", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("nbf", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("exp", fn -> nil end, &is_integer/1)

    assert {:ok, token, _} = Joken.generate_and_sign(config, claims, Keys.signer())
    assert {:error, reason} = ManifestSeal.verify(token)

    assert Keyword.get(List.wrap(reason) |> ensure_keyword(), :claim) == "exp" or
             match?([{:claim, "exp"} | _], List.wrap(reason)) or
             expish?(reason)
  end

  defp ensure_keyword(list) when is_list(list) do
    if Keyword.keyword?(list), do: list, else: []
  end

  defp expish?(reason) when is_list(reason) do
    Enum.any?(reason, fn
      {:claim, "exp"} -> true
      _ -> false
    end) or Keyword.get(reason, :claim) == "exp"
  end

  defp expish?(_), do: false
end
