defmodule Auth.Repo.Migrations.AddRelationOutbox do
  @moduledoc """
  x00d7: durable auth → world relation_changed outbox (same TX as social mutate).
  """
  use Ecto.Migration

  def up do
    create table(:relation_outbox, primary_key: false) do
      add :id, :uuid, null: false, default: fragment("gen_random_uuid()"), primary_key: true
      add :sub_a, :uuid, null: false
      add :sub_b, :uuid, null: false
      add :reason, :text, null: false
      add :affected_owner_subs, {:array, :uuid}, null: false, default: []
      add :status, :text, null: false, default: "pending"
      add :attempts, :integer, null: false, default: 0

      add :next_attempt_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")

      add :last_error, :text
      add :delivered_at, :utc_datetime_usec

      add :inserted_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")

      add :updated_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")
    end

    create index(:relation_outbox, [:status, :next_attempt_at],
             name: "relation_outbox_pending_due_index"
           )
  end

  def down do
    drop table(:relation_outbox)
  end
end
