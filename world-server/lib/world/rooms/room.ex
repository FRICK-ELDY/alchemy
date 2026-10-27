defmodule World.Rooms.Room do
  @moduledoc """
  Persistent room ledger row (x0028).

  Guarantees: `room_id` + `owner_sub` + DemoScene `scene` + `visibility` survive restart.
  Non-guarantees: Presence / physics pose; UGC edit history.
  """

  use Ash.Resource,
    otp_app: :world,
    domain: World.Domain,
    data_layer: AshSqlite.DataLayer

  sqlite do
    table "rooms"
    repo World.Repo
  end

  attributes do
    attribute :room_id, :string do
      primary_key? true
      allow_nil? false
      public? true
    end

    attribute :owner_sub, :string do
      allow_nil? false
      public? true
    end

    # DemoScene map: %{"revision" => …, "entries" => […], "locomotion" => %{…}}
    attribute :scene, :map do
      allow_nil? false
      public? true
    end

    attribute :visibility, :string do
      allow_nil? false
      public? true
      default "public"

      constraints match: ~r/^(lan|private|friends|friends_plus|followers|followers_plus|public)$/
    end

    create_timestamp :inserted_at
    update_timestamp :updated_at
  end

  # Uniqueness is the primary key on `room_id` (no redundant Ash identity).

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:room_id, :owner_sub, :scene, :visibility]
    end

    update :set_visibility do
      accept [:visibility]
    end

    read :get_by_room_id do
      get? true
      argument :room_id, :string, allow_nil?: false
      filter expr(room_id == ^arg(:room_id))
    end
  end

  code_interface do
    define :create, action: :create
    define :get_by_room_id, action: :get_by_room_id, args: [:room_id]
    define :set_visibility, action: :set_visibility
  end
end
