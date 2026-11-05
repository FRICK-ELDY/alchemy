defmodule Assets.CAS.OrphanGc do
  @moduledoc """
  Deletes CAS bytes that have no CasObject after grace (x0074).

  Candidates come from compensate / reserved reaper / local directory scan.
  Never runs on the upload compensate path — that path must not `CAS.delete`.

  Deletion uses quarantine → re-check CasObject/reservation → discard/restore
  so a concurrent PUT finalize cannot lose live bytes to TOCTOU.
  """

  use GenServer

  require Logger

  import Ecto.Query

  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.CAS.Local
  alias Assets.CAS.OrphanCandidate
  alias Assets.Quota.QuotaReservation
  alias Assets.Repo

  @call_timeout_ms 60_000

  @type gc_result :: %{deleted: non_neg_integer(), noted: non_neg_integer()}

  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec run() :: gc_result()
  def run do
    GenServer.call(__MODULE__, :run, @call_timeout_ms)
  end

  @doc false
  @spec run_now() :: gc_result()
  def run_now do
    do_run()
  end

  @impl true
  def init(_opts) do
    schedule()
    {:ok, %{}}
  end

  @impl true
  def handle_call(:run, _from, state) do
    {:reply, do_run(), state}
  end

  @impl true
  def handle_info(:gc, state) do
    result = do_run()

    Logger.info(
      inspect(%{
        event: "assets.cas_orphan_gc.completed",
        deleted: result.deleted,
        noted: result.noted
      })
    )

    schedule()
    {:noreply, state}
  end

  defp do_run do
    if Application.fetch_env!(:assets, :orphan_cas_gc_enabled) do
      noted = note_local_orphans()
      deleted = delete_ready_orphans()
      %{deleted: deleted, noted: noted}
    else
      %{deleted: 0, noted: 0}
    end
  end

  defp note_local_orphans do
    case Application.fetch_env!(:assets, :cas_backend) do
      :local -> Enum.reduce(Local.list_blake3_hexes(), 0, &note_local_orphan/2)
      _ -> 0
    end
  end

  defp note_local_orphan(blake3, acc) do
    if orphan_bytes?(blake3) and note_candidate(blake3) == :inserted do
      acc + 1
    else
      acc
    end
  end

  defp delete_ready_orphans do
    grace = Application.fetch_env!(:assets, :orphan_cas_grace_seconds)
    max = Application.fetch_env!(:assets, :orphan_cas_gc_max_per_run)
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)
    cutoff = DateTime.add(now, -grace, :second)

    from(c in OrphanCandidate,
      where: c.noticed_at <= ^cutoff,
      order_by: [asc: c.noticed_at],
      limit: ^max
    )
    |> Repo.all()
    |> Enum.reduce(0, fn candidate, acc ->
      case try_delete(candidate.blake3) do
        :deleted -> acc + 1
        :skipped -> acc
      end
    end)
  end

  defp try_delete(blake3) do
    cond do
      not is_nil(Repo.get(CasObject, blake3)) ->
        restore_sidecar_then_maybe_drop(blake3)

      reservation_open?(blake3) ->
        :skipped

      true ->
        case cas_present?(blake3) do
          :unknown ->
            :skipped

          # false: live may be gone while quarantine sidecar remains — still
          # enter delete_orphan (local/S3 quarantine treats q-only as :ok).
          _present_or_absent ->
            finish_delete_orphan(blake3)
        end
    end
  end

  # CasObject exists: restore/clear interrupted quarantine. Only drop the
  # candidate when restore succeeds — otherwise retry next run (S3 blip etc.).
  defp restore_sidecar_then_maybe_drop(blake3) do
    case CAS.restore_quarantine(blake3) do
      :ok ->
        drop_candidate(blake3)
        :skipped

      {:error, reason} ->
        Logger.warning(
          inspect(%{
            event: "assets.cas_orphan_gc.restore_failed",
            blake3: blake3,
            reason: inspect(reason)
          })
        )

        :skipped
    end
  end

  defp finish_delete_orphan(blake3) do
    case CAS.delete_orphan(blake3, fn -> still_orphan?(blake3) end) do
      :deleted ->
        drop_candidate(blake3)
        :deleted

      :missing ->
        drop_candidate(blake3)
        :skipped

      :aborted ->
        :skipped

      {:error, reason} ->
        Logger.warning(
          inspect(%{
            event: "assets.cas_orphan_gc.delete_failed",
            blake3: blake3,
            reason: inspect(reason)
          })
        )

        :skipped
    end
  end

  defp still_orphan?(blake3) do
    is_nil(Repo.get(CasObject, blake3)) and not reservation_open?(blake3)
  end

  defp orphan_bytes?(blake3) do
    still_orphan?(blake3) and cas_present?(blake3) == true
  end

  defp cas_present?(blake3) do
    case CAS.exists?(blake3) do
      true ->
        true

      false ->
        false

      {:error, reason} ->
        Logger.warning(
          inspect(%{
            event: "assets.cas_orphan_gc.exists_uncertain",
            blake3: blake3,
            reason: inspect(reason)
          })
        )

        :unknown
    end
  end

  defp reservation_open?(blake3) do
    Repo.exists?(from(r in QuotaReservation, where: r.blake3 == ^blake3))
  end

  defp note_candidate(blake3) do
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    {n, _} =
      Repo.insert_all(
        OrphanCandidate,
        [%{blake3: blake3, noticed_at: now}],
        on_conflict: :nothing,
        conflict_target: :blake3
      )

    if n == 1, do: :inserted, else: :existing
  end

  defp drop_candidate(blake3) do
    from(c in OrphanCandidate, where: c.blake3 == ^blake3)
    |> Repo.delete_all()

    :ok
  end

  defp schedule do
    interval_ms = Application.fetch_env!(:assets, :orphan_cas_gc_interval_ms)
    Process.send_after(self(), :gc, interval_ms)
  end
end
