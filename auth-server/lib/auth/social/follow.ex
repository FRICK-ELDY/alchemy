defmodule Auth.Social.Follow do
  @moduledoc """
  Directed follow edge (X-style). SSoT lives in auth; JWT never carries the graph.
  """

  use Ash.Resource,
    otp_app: :auth,
    domain: Auth.Domain,
    data_layer: AshPostgres.DataLayer

  postgres do
    table "follows"
    repo Auth.Repo

    references do
      reference :follower, on_delete: :delete
      reference :followee, on_delete: :delete
    end
  end

  attributes do
    uuid_primary_key :id

    create_timestamp :inserted_at
    update_timestamp :updated_at
  end

  relationships do
    belongs_to :follower, Auth.Accounts.User do
      allow_nil? false
      public? true
      attribute_writable? true
    end

    belongs_to :followee, Auth.Accounts.User do
      allow_nil? false
      public? true
      attribute_writable? true
    end
  end

  identities do
    identity :unique_follower_followee, [:follower_id, :followee_id]
  end

  actions do
    defaults [:read, :destroy]

    create :create do
      primary? true
      accept [:follower_id, :followee_id]

      validate attribute_does_not_equal(:follower_id, :followee_id),
        message: "cannot follow yourself"
    end

    read :get_edge do
      get? true
      argument :follower_id, :uuid, allow_nil?: false
      argument :followee_id, :uuid, allow_nil?: false
      filter expr(follower_id == ^arg(:follower_id) and followee_id == ^arg(:followee_id))
    end

    read :list_following do
      argument :follower_id, :uuid, allow_nil?: false
      filter expr(follower_id == ^arg(:follower_id))
    end

    read :list_followers do
      argument :followee_id, :uuid, allow_nil?: false
      filter expr(followee_id == ^arg(:followee_id))
    end
  end

  code_interface do
    define :create, action: :create
    define :get_edge, action: :get_edge, args: [:follower_id, :followee_id]
    define :list_following, action: :list_following, args: [:follower_id]
    define :list_followers, action: :list_followers, args: [:followee_id]
    define :destroy, action: :destroy
  end
end
