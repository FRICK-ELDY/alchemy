defmodule Assets.Quota.QuotaLedger do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:owner_sub, :string, autogenerate: false}
  @foreign_key_type :string

  schema "quota_ledgers" do
    field :used_bytes, :integer, default: 0
    field :limit_bytes, :integer

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(ledger, attrs) do
    ledger
    |> cast(attrs, [:owner_sub, :used_bytes, :limit_bytes])
    |> validate_required([:owner_sub, :used_bytes, :limit_bytes])
    |> validate_number(:used_bytes, greater_than_or_equal_to: 0)
    |> validate_number(:limit_bytes, greater_than: 0)
  end
end
