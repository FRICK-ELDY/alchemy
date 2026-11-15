defmodule World.SpawnTicketTest do
  use World.DataCase, async: false

  alias World.RoomParticipants
  alias World.SpawnTicket
  alias World.TestSpawnTicketHelpers

  setup do
    %{signer: signer} = TestSpawnTicketHelpers.install_test_assets_jwks!()
    RoomParticipants.ensure_table!()
    SpawnTicket.ensure_consume_table!()
    %{signer: signer}
  end

  test "authorize_and_claim accepts valid ticket once", %{signer: signer} do
    sub = "11111111-1111-1111-1111-111111111111"
    room_id = "spawn-ok"
    blake3 = String.duplicate("ab", 32)
    RoomParticipants.note(room_id, sub, System.system_time(:second) + 300)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [blake3]
      )

    assert {:ok, claims} = SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
    assert claims["sub"] == sub

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
  end

  test "missing ticket is required", %{signer: _signer} do
    assert {:error, :spawn_ticket_required} =
             SpawnTicket.authorize_and_claim(nil, "room", String.duplicate("ab", 32))
  end

  test "wrong room is invalid", %{signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    blake3 = String.duplicate("cd", 32)
    RoomParticipants.note("room-a", sub, System.system_time(:second) + 300)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: "room-a",
        blake3s: [blake3]
      )

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, "room-b", blake3)
  end

  test "blake3 not in ticket is invalid", %{signer: signer} do
    sub = "33333333-3333-3333-3333-333333333333"
    room_id = "room-b3"
    RoomParticipants.note(room_id, sub, System.system_time(:second) + 300)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [String.duplicate("11", 32)]
      )

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, room_id, String.duplicate("22", 32))
  end

  test "expired ticket is invalid", %{signer: signer} do
    sub = "44444444-4444-4444-4444-444444444444"
    room_id = "room-exp"
    blake3 = String.duplicate("ef", 32)
    RoomParticipants.note(room_id, sub, System.system_time(:second) + 300)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [blake3],
        exp: System.system_time(:second) - 10
      )

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
  end

  test "non-participant is invalid", %{signer: signer} do
    sub = "55555555-5555-5555-5555-555555555555"
    room_id = "room-np"
    blake3 = String.duplicate("aa", 32)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [blake3]
      )

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
  end

  test "release_claim allows CAS retry", %{signer: signer} do
    sub = "66666666-6666-6666-6666-666666666666"
    room_id = "room-rel"
    blake3 = String.duplicate("bb", 32)
    RoomParticipants.note(room_id, sub, System.system_time(:second) + 300)

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [blake3]
      )

    assert {:ok, claims} = SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
    assert :ok = SpawnTicket.release_claim(claims["jti"], blake3)
    assert {:ok, _} = SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
  end

  test "consume stores ticket exp; prune while JWT valid does not reopen reuse", %{
    signer: signer
  } do
    sub = "77777777-7777-7777-7777-777777777777"
    room_id = "room-exp-retain"
    blake3 = String.duplicate("cc", 32)
    RoomParticipants.note(room_id, sub, System.system_time(:second) + 3_600)
    # Long-lived ticket: old fixed-retain(120s) prune would have wiped the claim early.
    exp = System.system_time(:second) + 3_600

    ticket =
      TestSpawnTicketHelpers.spawn_ticket(signer,
        sub: sub,
        room_id: room_id,
        blake3s: [blake3],
        exp: exp
      )

    assert {:ok, claims} = SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
    assert claims["exp"] == exp
    assert {:ok, ^exp} = SpawnTicket.consume_exp(claims["jti"], blake3)

    # Prune must not drop rows whose stored value is still-future ticket exp.
    assert :ok = SpawnTicket.force_prune_expired_claims!()
    assert {:ok, ^exp} = SpawnTicket.consume_exp(claims["jti"], blake3)

    assert {:error, :spawn_ticket_invalid} =
             SpawnTicket.authorize_and_claim(ticket, room_id, blake3)
  end
end
