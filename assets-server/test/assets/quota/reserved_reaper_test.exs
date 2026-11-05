defmodule Assets.Quota.ReservedReaperTest do
  use Assets.DataCase, async: false

  alias Assets.Blobs
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.Quota.QuotaLedger
  alias Assets.Quota.QuotaReservation
  alias Assets.Quota.ReservedReaper
  alias Assets.Repo

  @sub "user-reaper-a"

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))

    prev_ttl = Application.get_env(:assets, :quota_reservation_ttl_seconds)
    Application.put_env(:assets, :quota_reservation_ttl_seconds, 60)

    on_exit(fn ->
      Application.put_env(:assets, :quota_reservation_ttl_seconds, prev_ttl)
    end)

    :ok
  end

  test "reaps expired reservation and clears reserved_bytes" do
    ensure_ledger!(@sub, reserved: 11)
    plant_reservation!(@sub, blake3_of("stale-one"), 11, seconds_ago: 120)

    assert %{reaped: 1} = ReservedReaper.run_now()
    assert Blobs.quota(@sub).reserved_bytes == 0
    assert Repo.get_by(QuotaReservation, owner_sub: @sub) == nil
  end

  test "keeps reservations inside TTL" do
    ensure_ledger!(@sub, reserved: 7)
    plant_reservation!(@sub, blake3_of("fresh-one"), 7, seconds_ago: 1)

    assert %{reaped: 0} = ReservedReaper.run_now()
    assert Blobs.quota(@sub).reserved_bytes == 7
    assert %QuotaReservation{} = Repo.get_by(QuotaReservation, owner_sub: @sub)
  end

  test "partially reaps only expired of two concurrent reservations" do
    ensure_ledger!(@sub, reserved: 20)
    plant_reservation!(@sub, blake3_of("old-blob"), 9, seconds_ago: 120)
    plant_reservation!(@sub, blake3_of("new-blob"), 11, seconds_ago: 1)

    assert %{reaped: 1} = ReservedReaper.run_now()
    assert Blobs.quota(@sub).reserved_bytes == 11

    left = Repo.all(QuotaReservation)
    assert length(left) == 1
    assert hd(left).blake3 == blake3_of("new-blob")
  end

  test "successful upload clears reservation row" do
    assert {:ok, %CasObject{}, :created} =
             Blobs.upload(@sub, "upload-clears-reservation", "text/plain")

    assert Blobs.quota(@sub).reserved_bytes == 0
    assert Repo.aggregate(QuotaReservation, :count) == 0
  end

  test "reaper notes orphan candidate when CAS bytes exist without CasObject" do
    data = "orphan-after-reaper"
    blake3 = blake3_of(data)
    assert :created = CAS.put_if_absent(blake3, data)

    ensure_ledger!(@sub, reserved: byte_size(data))
    plant_reservation!(@sub, blake3, byte_size(data), seconds_ago: 120)

    assert %{reaped: 1} = ReservedReaper.run_now()
    assert Repo.get(CasObject, blake3) == nil
    assert CAS.exists?(blake3)
  end

  defp ensure_ledger!(owner_sub, reserved: reserved) do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    %QuotaLedger{}
    |> QuotaLedger.changeset(%{
      owner_sub: owner_sub,
      used_bytes: 0,
      reserved_bytes: reserved,
      limit_bytes: limit
    })
    |> Repo.insert!(
      on_conflict: [
        set: [reserved_bytes: reserved, used_bytes: 0, limit_bytes: limit]
      ],
      conflict_target: :owner_sub
    )
  end

  defp plant_reservation!(owner_sub, blake3, bytes, seconds_ago: ago) do
    at =
      DateTime.utc_now()
      |> DateTime.add(-ago, :second)
      |> DateTime.truncate(:microsecond)

    %QuotaReservation{}
    |> QuotaReservation.changeset(%{
      owner_sub: owner_sub,
      blake3: blake3,
      bytes: bytes,
      reserved_at: at
    })
    |> Repo.insert!()
  end

  defp blake3_of(data), do: Blobs.blake3_hex(data)
end
