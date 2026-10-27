defmodule Auth.Social.Friendship do
  @moduledoc """
  Symmetric friendship with pending requests. Pair is stored as ordered `(sub_lo, sub_hi)`.
  """

  use Ash.Resource,
    otp_app: :auth,
    domain: Auth.Domain,
    data_layer: AshPostgres.DataLayer

  postgres do
    table "friendships"
    repo Auth.Repo

    references do
      reference :requester, on_delete: :delete
    end
  end

  attributes do
    uuid_primary_key :id

    attribute :sub_lo, :uuid do
      allow_nil? false
      public? true
    end

    attribute :sub_hi, :uuid do
      allow_nil? false
      public? true
    end

    attribute :state, :atom do
      constraints one_of: [:pending, :accepted]
      allow_nil? false
      public? true
    end

    create_timestamp :inserted_at
    update_timestamp :updated_at
  end

  relationships do
    belongs_to :requester, Auth.Accounts.User do
      allow_nil? false
      public? true
      attribute_writable? true
    end
  end

  identities do
    identity :unique_pair, [:sub_lo, :sub_hi]
  end

  actions do
    defaults [:read, :destroy]

    create :create do
      primary? true
      accept [:sub_lo, :sub_hi, :requester_id, :state]
      validate {Auth.Social.Friendship.ValidatePair, []}
    end

    update :accept do
      accept []
      change set_attribute(:state, :accepted)
      change filter(expr(state == :pending))
    end

    read :get_pair do
      get? true
      argument :sub_lo, :uuid, allow_nil?: false
      argument :sub_hi, :uuid, allow_nil?: false
      filter expr(sub_lo == ^arg(:sub_lo) and sub_hi == ^arg(:sub_hi))
    end

    read :list_accepted_for do
      argument :user_id, :uuid, allow_nil?: false
      filter expr(state == :accepted and (sub_lo == ^arg(:user_id) or sub_hi == ^arg(:user_id)))
    end
  end

  code_interface do
    define :create, action: :create
    define :get_pair, action: :get_pair, args: [:sub_lo, :sub_hi]
    define :list_accepted_for, action: :list_accepted_for, args: [:user_id]
    define :accept, action: :accept
    define :destroy, action: :destroy
  end
end
