defmodule Auth.Social.Friendship.ValidatePair do
  @moduledoc false
  use Ash.Resource.Validation

  @impl true
  def init(opts), do: {:ok, opts}

  @impl true
  def validate(changeset, _opts, _context) do
    lo = Ash.Changeset.get_attribute(changeset, :sub_lo)
    hi = Ash.Changeset.get_attribute(changeset, :sub_hi)
    requester = Ash.Changeset.get_attribute(changeset, :requester_id)

    cond do
      is_nil(lo) or is_nil(hi) or is_nil(requester) ->
        :ok

      lo >= hi ->
        {:error, field: :sub_lo, message: "must be less than sub_hi"}

      requester not in [lo, hi] ->
        {:error, field: :requester_id, message: "must be one of the pair"}

      true ->
        :ok
    end
  end
end
