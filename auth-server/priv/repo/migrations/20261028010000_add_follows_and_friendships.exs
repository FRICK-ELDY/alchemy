defmodule Auth.Repo.Migrations.AddFollowsAndFriendships do
  use Ecto.Migration

  def up do
    create table(:follows, primary_key: false) do
      add :id, :uuid, null: false, default: fragment("gen_random_uuid()"), primary_key: true

      add :follower_id, references(:users, type: :uuid, on_delete: :delete_all), null: false
      add :followee_id, references(:users, type: :uuid, on_delete: :delete_all), null: false

      add :inserted_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")

      add :updated_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")
    end

    create unique_index(:follows, [:follower_id, :followee_id],
             name: "follows_unique_follower_followee_index"
           )

    create index(:follows, [:followee_id], name: "follows_followee_id_index")

    create table(:friendships, primary_key: false) do
      add :id, :uuid, null: false, default: fragment("gen_random_uuid()"), primary_key: true
      add :sub_lo, :uuid, null: false
      add :sub_hi, :uuid, null: false
      add :requester_id, references(:users, type: :uuid, on_delete: :delete_all), null: false
      add :state, :text, null: false

      add :inserted_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")

      add :updated_at, :utc_datetime_usec,
        null: false,
        default: fragment("(now() AT TIME ZONE 'utc')")
    end

    create unique_index(:friendships, [:sub_lo, :sub_hi], name: "friendships_unique_pair_index")

    create index(:friendships, [:state], name: "friendships_state_index")
  end

  def down do
    drop table(:friendships)
    drop table(:follows)
  end
end
