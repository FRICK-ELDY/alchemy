defmodule World.Repo.Migrations.RelationEvents do
  @moduledoc """
  x00d7: idempotent inbox for auth relation_changed event_id.
  """
  use Ecto.Migration

  def up do
    execute("""
    CREATE TABLE IF NOT EXISTS relation_events (
      event_id TEXT PRIMARY KEY NOT NULL,
      consumed_at TEXT NOT NULL
    );
    """)
  end

  def down do
    execute("DROP TABLE IF EXISTS relation_events;")
  end
end
