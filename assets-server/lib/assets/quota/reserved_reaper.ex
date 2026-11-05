defmodule Assets.Quota.ReservedReaper do
  @moduledoc """
  Periodically releases TTL-expired `quota_reservations` (x0074).

  Same shape as `Auth.TokenCleanup`: GenServer + `Process.send_after`.
  Upload compensate remains the fast path; this covers crash after reserve.
  """

  use GenServer

  require Logger

  import Ecto.Query

  alias Assets.Blobs
  alias Assets.Blobs.CasObject
  alias Assets.Quota.QuotaLedger
  alias Assets.Quota.QuotaReservation
  alias Assets.Repo

  @type reap_result :: %{reaped: non_neg_integer()}

  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec run() :: reap_result()
  def run do
    GenServer.call(__MODULE__, :run)
  end

  @doc false
  @spec run_now() :: reap_result()
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
  def handle_info(:reap, state) do
    result = do_run()

    Logger.info(
      inspect(%{
        event: "assets.quota_reserved_reaper.completed",
        reaped: result.reaped
      })
    )

    schedule()
    {:noreply, state}
  end

  defp do_run do
    ttl = Application.fetch_env!(:assets, :quota_reservation_ttl_seconds)
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)
    cutoff = DateTime.add(now, -ttl, :second)

    expired =
      from(r in QuotaReservation,
        where: r.reserved_at < ^cutoff,
        order_by: [asc: r.reserved_at]
      )
      |> Repo.all()

    reaped =
      Enum.reduce(expired, 0, fn reservation, acc ->
        case reap_one(reservation) do
          :reaped -> acc + 1
          :skipped -> acc
        end
      end)

    %{reaped: reaped}
  end

  defp reap_one(%QuotaReservation{} = reservation) do
    result =
      Repo.transaction(fn ->
        {n, _} =
          from(r in QuotaReservation, where: r.id == ^reservation.id)
          |> Repo.delete_all()

        if n == 1 do
          release_reserved_bytes!(reservation.owner_sub, reservation.bytes)
          :reaped
        else
          :skipped
        end
      end)

    case result do
      {:ok, :reaped} ->
        maybe_note_orphan(reservation.blake3)
        :reaped

      {:ok, :skipped} ->
        :skipped

      {:error, reason} ->
        Logger.warning(
          inspect(%{
            event: "assets.quota_reserved_reaper.failed",
            blake3: reservation.blake3,
            reason: inspect(reason)
          })
        )

        :skipped
    end
  end

  defp release_reserved_bytes!(owner_sub, size) when size >= 0 do
    {_n, _} =
      from(q in QuotaLedger,
        where: q.owner_sub == ^owner_sub and q.reserved_bytes >= ^size
      )
      |> Repo.update_all(inc: [reserved_bytes: -size])

    # n may be 0 if ledger already lower (partial race) — row delete still stands.
    :ok
  end

  defp maybe_note_orphan(blake3) do
    if is_nil(Repo.get(CasObject, blake3)) do
      Blobs.note_orphan_candidate(blake3)
    else
      :ok
    end
  end

  defp schedule do
    interval_ms = Application.fetch_env!(:assets, :quota_reserved_reaper_interval_ms)
    Process.send_after(self(), :reap, interval_ms)
  end
end
