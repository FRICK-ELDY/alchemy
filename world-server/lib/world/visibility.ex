defmodule World.Visibility do
  @moduledoc """
  Room visibility levels (x0033–x0036).

  Order: lan → private → friends → friends_plus →
  followers → followers_plus → public.
  """

  @levels ~w(lan private friends friends_plus followers followers_plus public)

  @spec levels() :: [String.t()]
  def levels, do: @levels

  @spec valid?(term()) :: boolean()
  def valid?(v) when is_binary(v), do: v in @levels
  def valid?(_), do: false

  @spec parse(term()) :: {:ok, String.t()} | {:error, :invalid_visibility}
  def parse(v) when is_binary(v) do
    if valid?(v), do: {:ok, v}, else: {:error, :invalid_visibility}
  end

  def parse(v) when is_atom(v), do: parse(Atom.to_string(v))
  def parse(_), do: {:error, :invalid_visibility}

  @doc "Levels that need auth social-graph S2S."
  @spec needs_social?(String.t()) :: boolean()
  def needs_social?(v) when v in ~w(friends friends_plus followers followers_plus), do: true
  def needs_social?(_), do: false
end
