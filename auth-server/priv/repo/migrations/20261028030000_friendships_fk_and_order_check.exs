defmodule Auth.Repo.Migrations.FriendshipsFkAndOrderCheck do
  use Ecto.Migration

  def up do
    # sub_lo / sub_hi must reference users; pair must stay ordered (app pair/2 + DB).
    execute("""
    ALTER TABLE friendships
      ADD CONSTRAINT friendships_sub_lo_fkey
        FOREIGN KEY (sub_lo) REFERENCES users(id) ON DELETE CASCADE,
      ADD CONSTRAINT friendships_sub_hi_fkey
        FOREIGN KEY (sub_hi) REFERENCES users(id) ON DELETE CASCADE,
      ADD CONSTRAINT friendships_ordered_check
        CHECK (sub_lo < sub_hi)
    """)
  end

  def down do
    execute("""
    ALTER TABLE friendships
      DROP CONSTRAINT IF EXISTS friendships_ordered_check,
      DROP CONSTRAINT IF EXISTS friendships_sub_hi_fkey,
      DROP CONSTRAINT IF EXISTS friendships_sub_lo_fkey
    """)
  end
end
