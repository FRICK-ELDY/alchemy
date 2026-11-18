defmodule Auth.WorldNotify.Outbox do
  @moduledoc """
  Durable relation_changed outbox (x00d7).

  Guarantees: rows inserted in the same `Repo.transaction` as social graph mutate;
  pending rows are delivered until world returns 2xx; `claim_due/1` uses
  `FOR UPDATE SKIP LOCKED` so concurrent drains do not double-claim;
  delivered rows older than retention are purged (not an unbounded audit log).
  Non-guarantees: exact-once beyond world `event_id` idempotency; ordering across pairs.
  """

  use Ecto.Schema

  require Logger

  import Ecto.Changeset
  import Ecto.Query

  alias Auth.Repo

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  @statuses ~w(pending delivered)
  # Lease while delivering so kick+poll do not race the same row.
  @claim_lease_ms 30_000
  @default_delivered_retention_days 7

  schema "relation_outbox" do
    field :sub_a, Ecto.UUID
    field :sub_b, Ecto.UUID
    field :reason, :string
    field :affected_owner_subs, {:array, Ecto.UUID}, default: []
    field :status, :string, default: "pending"
    field :attempts, :integer, default: 0
    field :next_attempt_at, :utc_datetime_usec
    field :last_error, :string
    field :delivered_at, :utc_datetime_usec

    timestamps(type: :utc_datetime_usec)
  end

  @type t :: %__MODULE__{}

  @doc """
  Insert a pending outbox row. Must run inside the social mutate transaction.
  """
  @spec insert!(String.t(), String.t(), String.t(), [String.t()]) :: t()
  def insert!(sub_a, sub_b, reason, affected_owner_subs)
      when is_binary(sub_a) and is_binary(sub_b) and is_binary(reason) and
             is_list(affected_owner_subs) do
    now = DateTime.utc_now()

    %__MODULE__{}
    |> cast(
      %{
        sub_a: sub_a,
        sub_b: sub_b,
        reason: reason,
        affected_owner_subs: affected_owner_subs,
        status: "pending",
        attempts: 0,
        next_attempt_at: now
      },
      [:sub_a, :sub_b, :reason, :affected_owner_subs, :status, :attempts, :next_attempt_at]
    )
    |> validate_required([
      :sub_a,
      :sub_b,
      :reason,
      :affected_owner_subs,
      :status,
      :next_attempt_at
    ])
    |> validate_inclusion(:status, @statuses)
    |> Repo.insert!()
  end

  @spec get(String.t()) :: t() | nil
  def get(id) when is_binary(id), do: Repo.get(__MODULE__, id)
  def get(_), do: nil

  @doc """
  Claim due pending rows under row locks. Bumps `attempts` and advances `next_attempt_at`
  by a short lease so a crashed deliverer is visible in metrics and reclaims after the lease;
  concurrent drainers skip locked rows. TX failure returns `[]` (does not raise).
  """
  @spec claim_due(pos_integer()) :: [t()]
  def claim_due(limit \\ 32) when is_integer(limit) and limit > 0 do
    now = DateTime.utc_now()
    lease_until = DateTime.add(now, @claim_lease_ms, :millisecond)

    case Repo.transaction(fn ->
           ids =
             from(r in __MODULE__,
               where: r.status == "pending" and r.next_attempt_at <= ^now,
               order_by: [asc: r.next_attempt_at, asc: r.inserted_at],
               limit: ^limit,
               lock: "FOR UPDATE SKIP LOCKED",
               select: r.id
             )
             |> Repo.all()

           if ids == [] do
             []
           else
             {_, _} =
               from(r in __MODULE__, where: r.id in ^ids)
               |> Repo.update_all(
                 inc: [attempts: 1],
                 set: [next_attempt_at: lease_until, updated_at: now]
               )

             from(r in __MODULE__, where: r.id in ^ids, order_by: [asc: r.inserted_at])
             |> Repo.all()
           end
         end) do
      {:ok, rows} when is_list(rows) ->
        rows

      {:error, reason} ->
        Logger.warning("WorldNotify.Outbox.claim_due failed: #{inspect(reason)}")
        []
    end
  end

  @spec mark_delivered(t()) :: t()
  def mark_delivered(%__MODULE__{} = row) do
    now = DateTime.utc_now()

    row
    |> change(%{status: "delivered", delivered_at: now, last_error: nil, updated_at: now})
    |> Repo.update!()
  end

  @doc """
  Schedule a retry after a failed deliver. `attempts` is already bumped by `claim_due/1`.
  """
  @spec mark_retry(t(), term(), non_neg_integer()) :: t()
  def mark_retry(%__MODULE__{} = row, error, backoff_ms)
      when is_integer(backoff_ms) and backoff_ms >= 0 do
    now = DateTime.utc_now()
    next = DateTime.add(now, backoff_ms, :millisecond)
    msg = error |> inspect() |> String.slice(0, 500)

    row
    |> change(%{
      next_attempt_at: next,
      last_error: msg,
      updated_at: now
    })
    |> Repo.update!()
  end

  @doc "Delete delivered rows older than `days` (default 7). Not an audit log."
  @spec purge_delivered(pos_integer()) :: non_neg_integer()
  def purge_delivered(days \\ @default_delivered_retention_days)
      when is_integer(days) and days > 0 do
    cutoff = DateTime.add(DateTime.utc_now(), -days * 86_400, :second)

    {count, _} =
      from(r in __MODULE__,
        where: r.status == "delivered" and r.delivered_at < ^cutoff
      )
      |> Repo.delete_all()

    count
  end

  @spec payload(t()) :: map()
  def payload(%__MODULE__{} = row) do
    %{
      event_id: row.id,
      sub_a: row.sub_a,
      sub_b: row.sub_b,
      reason: row.reason,
      affected_owner_subs: Enum.map(row.affected_owner_subs, &to_string/1)
    }
  end
end
