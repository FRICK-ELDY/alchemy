defmodule Assets.SpawnTicketTest do
  use Assets.DataCase, async: false, cas: true

  alias Assets.Blobs
  alias Assets.SpawnTicket

  @sub "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"

  test "generate issues JWT with expected claims" do
    blake3 = String.duplicate("ab", 32)
    assert {:ok, token, ttl} = SpawnTicket.generate(@sub, "local", [blake3])
    assert ttl == 60
    assert is_binary(token)
    refute String.contains?(token, "BEGIN PRIVATE")

    {:ok, claims} = Joken.peek_claims(token)
    assert claims["typ"] == SpawnTicket.typ()
    assert claims["aud"] == SpawnTicket.audience()
    assert claims["sub"] == @sub
    assert claims["room_id"] == "local"
    assert claims["blake3s"] == [blake3]
    assert is_binary(claims["jti"])
  end

  test "issue_for_grantee requires Grant" do
    blake3 = String.duplicate("cd", 32)
    assert {:error, :no_grant} = SpawnTicket.issue_for_grantee(@sub, "local", [blake3])
  end

  test "issue_for_grantee succeeds after upload Grant" do
    {:ok, cas_obj, _} = Blobs.upload(@sub, "spawn-ticket-mesh", "application/octet-stream")
    assert {:ok, token, _ttl} = SpawnTicket.issue_for_grantee(@sub, "local", [cas_obj.blake3])
    assert is_binary(token)
  end

  test "rejects invalid room_id and empty blake3s" do
    blake3 = String.duplicate("ab", 32)
    assert {:error, :invalid_room_id} = SpawnTicket.generate(@sub, "bad/id", [blake3])
    assert {:error, :blake3s_required} = SpawnTicket.generate(@sub, "local", [])
    assert {:error, :invalid_blake3} = SpawnTicket.generate(@sub, "local", ["nope"])
  end
end
