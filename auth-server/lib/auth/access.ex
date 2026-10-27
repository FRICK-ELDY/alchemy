defmodule Auth.Access do
  @moduledoc """
  Room visibility predicate against the social graph (x0033–x0035).

  `lan` is path-based and evaluated by world-server, not here.
  `private` is owner-only (also enforced in world; included for S2S completeness).
  """

  alias Auth.Social

  @visibilities ~w(lan private friends friends_plus followers followers_plus public)a

  @spec visibilities() :: [atom()]
  def visibilities, do: @visibilities

  @spec parse_visibility(String.t() | atom()) :: {:ok, atom()} | {:error, :invalid_visibility}
  def parse_visibility(v) when is_atom(v) do
    if v in @visibilities, do: {:ok, v}, else: {:error, :invalid_visibility}
  end

  def parse_visibility(v) when is_binary(v) do
    parse_visibility(String.to_existing_atom(v))
  rescue
    ArgumentError -> {:error, :invalid_visibility}
  end

  def parse_visibility(_), do: {:error, :invalid_visibility}

  @doc """
  Returns whether `viewer_sub` may enter a room owned by `owner_sub` at `visibility`.

  Owner always allowed. `lan` returns true (world ANDs LAN path). `public` true.
  """
  @spec allowed?(String.t(), String.t(), atom()) :: boolean()
  def allowed?(owner_sub, viewer_sub, visibility)
      when is_binary(owner_sub) and is_binary(viewer_sub) and is_atom(visibility) do
    owner_sub == viewer_sub or allowed_for_visibility?(owner_sub, viewer_sub, visibility)
  end

  def allowed?(_, _, _), do: false

  # Path gate for `:lan` is world's job; relationship gate is open.
  defp allowed_for_visibility?(_owner_sub, _viewer_sub, :public), do: true
  defp allowed_for_visibility?(_owner_sub, _viewer_sub, :lan), do: true
  defp allowed_for_visibility?(_owner_sub, _viewer_sub, :private), do: false

  defp allowed_for_visibility?(owner_sub, viewer_sub, :friends),
    do: Social.friends?(owner_sub, viewer_sub)

  defp allowed_for_visibility?(owner_sub, viewer_sub, :friends_plus),
    do: Social.friends_plus?(owner_sub, viewer_sub)

  defp allowed_for_visibility?(owner_sub, viewer_sub, :followers),
    do: Social.follower?(owner_sub, viewer_sub)

  defp allowed_for_visibility?(owner_sub, viewer_sub, :followers_plus),
    do: Social.followers_plus?(owner_sub, viewer_sub)

  defp allowed_for_visibility?(_owner_sub, _viewer_sub, _visibility), do: false
end
