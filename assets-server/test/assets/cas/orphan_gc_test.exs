defmodule Assets.CAS.OrphanGcTest do
  use Assets.DataCase, async: false, cas: true

  alias Assets.Blobs
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.CAS.Local
  alias Assets.CAS.OrphanCandidate
  alias Assets.CAS.OrphanGc
  alias Assets.Quota.QuotaLedger
  alias Assets.Quota.QuotaReservation
  alias Assets.Repo

  @sub_a "user-orphan-a"
  @sub_b "user-orphan-b"

  setup do
    prev_grace = Application.get_env(:assets, :orphan_cas_grace_seconds)
    Application.put_env(:assets, :orphan_cas_grace_seconds, 0)

    on_exit(fn ->
      Application.put_env(:assets, :orphan_cas_grace_seconds, prev_grace)
    end)

    :ok
  end

  test "deletes orphan bytes after grace when no CasObject and no reservation" do
    data = "orphan-gc-delete-me"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    assert {:ok, _} =
             Repo.insert(
               OrphanCandidate.changeset(%OrphanCandidate{}, %{
                 blake3: blake3,
                 noticed_at: now
               })
             )

    assert %{deleted: 1} = OrphanGc.run_now()
    assert CAS.exists?(blake3) == false
    assert Repo.get(OrphanCandidate, blake3) == nil
  end

  test "does not delete while a reservation is open" do
    data = "orphan-kept-by-reservation"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    ensure_ledger!(@sub_a, reserved: byte_size(data))

    %QuotaReservation{}
    |> QuotaReservation.changeset(%{
      owner_sub: @sub_a,
      blake3: blake3,
      bytes: byte_size(data),
      reserved_at: DateTime.utc_now() |> DateTime.truncate(:microsecond)
    })
    |> Repo.insert!()

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    assert {:ok, _} =
             Repo.insert(
               OrphanCandidate.changeset(%OrphanCandidate{}, %{
                 blake3: blake3,
                 noticed_at: now
               })
             )

    assert %{deleted: 0} = OrphanGc.run_now()
    assert CAS.exists?(blake3)
  end

  test "does not delete when CasObject exists (shared peer)" do
    data = "shared-peer-bytes"

    assert {:ok, %CasObject{blake3: blake3}, :created} =
             Blobs.upload(@sub_a, data, "application/octet-stream")

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    Repo.insert(
      OrphanCandidate.changeset(%OrphanCandidate{}, %{blake3: blake3, noticed_at: now}),
      on_conflict: :nothing,
      conflict_target: :blake3
    )

    assert %{deleted: 0} = OrphanGc.run_now()
    assert CAS.exists?(blake3)
    assert Repo.get(CasObject, blake3)
  end

  test "finalize failure still leaves CAS until GC (compensate does not delete)" do
    data = "finalize-fail-then-gc"
    blake3 = Blobs.blake3_hex(data)
    Application.put_env(:assets, :fail_finalize_blake3, blake3)

    on_exit(fn -> Application.delete_env(:assets, :fail_finalize_blake3) end)

    assert {:error, :forced_finalize_failure} =
             Blobs.upload(@sub_b, data, "application/octet-stream")

    assert Blobs.quota(@sub_b).reserved_bytes == 0
    assert Repo.get(CasObject, blake3) == nil
    assert CAS.exists?(blake3)
    assert Repo.get(OrphanCandidate, blake3)

    assert %{deleted: 1} = OrphanGc.run_now()
    assert CAS.exists?(blake3) == false
  end

  test "local scan notes orphan files without candidate rows" do
    data = "scan-discovered-orphan"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    assert %{noted: 1, deleted: 1} = OrphanGc.run_now()
    assert CAS.exists?(blake3) == false
  end

  test "delete_orphan restores when re-check says referenced" do
    data = "toc-tou-restore"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    assert :aborted = CAS.delete_orphan(blake3, fn -> false end)
    assert CAS.exists?(blake3) == true
    refute File.exists?(Local.quarantine_path(blake3))
  end

  test "delete_orphan discards quarantine when still orphan" do
    data = "toc-tou-discard"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    assert :deleted = CAS.delete_orphan(blake3, fn -> true end)
    assert CAS.exists?(blake3) == false
    refute File.exists?(Local.quarantine_path(blake3))
  end

  test "resumes interrupted quarantine (live gone, sidecar left) and discards" do
    data = "interrupted-gc-discard"
    blake3 = Blobs.blake3_hex(data)
    assert :created = CAS.put_if_absent(blake3, data)

    assert :ok = Local.quarantine(blake3)
    refute File.exists?(Local.path_for(blake3))
    assert File.exists?(Local.quarantine_path(blake3))

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    assert {:ok, _} =
             Repo.insert(
               OrphanCandidate.changeset(%OrphanCandidate{}, %{
                 blake3: blake3,
                 noticed_at: now
               })
             )

    assert %{deleted: 1} = OrphanGc.run_now()
    assert CAS.exists?(blake3) == false
    refute File.exists?(Local.quarantine_path(blake3))
    assert Repo.get(OrphanCandidate, blake3) == nil
  end

  test "resumes interrupted quarantine when CasObject appeared — restores live" do
    data = "interrupted-gc-restore"

    assert {:ok, %CasObject{blake3: blake3}, :created} =
             Blobs.upload(@sub_a, data, "application/octet-stream")

    # Simulate crash after quarantine while CasObject already exists.
    assert :ok = Local.quarantine(blake3)
    refute File.exists?(Local.path_for(blake3))
    assert File.exists?(Local.quarantine_path(blake3))

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    Repo.insert(
      OrphanCandidate.changeset(%OrphanCandidate{}, %{blake3: blake3, noticed_at: now}),
      on_conflict: :nothing,
      conflict_target: :blake3
    )

    assert %{deleted: 0} = OrphanGc.run_now()
    assert CAS.exists?(blake3) == true
    refute File.exists?(Local.quarantine_path(blake3))
    assert Repo.get(OrphanCandidate, blake3) == nil
  end

  test "keeps candidate when CasObject restore_quarantine fails" do
    data = "restore-fail-keeps-candidate"

    assert {:ok, %CasObject{blake3: blake3}, :created} =
             Blobs.upload(@sub_a, data, "application/octet-stream")

    assert :ok = Local.quarantine(blake3)
    # Make restore fail: quarantine path present but not a renameable file.
    q = Local.quarantine_path(blake3)
    File.rm!(q)
    File.mkdir_p!(q)

    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    assert {:ok, _} =
             Repo.insert(
               OrphanCandidate.changeset(%OrphanCandidate{}, %{
                 blake3: blake3,
                 noticed_at: now
               })
             )

    assert %{deleted: 0} = OrphanGc.run_now()
    assert Repo.get(OrphanCandidate, blake3)
    # Cleanup directory sidecar so later tests are not confused.
    File.rm_rf!(q)
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
end
