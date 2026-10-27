defmodule Auth.Repo.Migrations.FriendshipsRequesterInPairCheck do
  use Ecto.Migration

  def up do
    execute("""
    ALTER TABLE friendships
      ADD CONSTRAINT friendships_requester_in_pair_check
        CHECK (requester_id = sub_lo OR requester_id = sub_hi)
    """)
  end

  def down do
    execute("""
    ALTER TABLE friendships
      DROP CONSTRAINT IF EXISTS friendships_requester_in_pair_check
    """)
  end
end
