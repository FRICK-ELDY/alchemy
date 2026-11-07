defmodule World.Visibility do
  @moduledoc """
  Room visibility levels (x0033–x0036).

  Order (closed → open): lan → private → friends → friends_plus →
  followers → followers_plus → public.

  x007e: moving toward a **lower** rank (more closed) is a visibility *narrow*
  and triggers Room key rotation.
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

  @doc "0 = most closed (`lan`), higher = more open (`public`)."
  @spec rank(String.t()) :: non_neg_integer() | nil
  def rank(v) when is_binary(v) do
    Enum.find_index(@levels, &(&1 == v))
  end

  def rank(_), do: nil

  @doc "True when `new_vis` is strictly more closed than `old_vis` (x007e rotate)."
  @spec narrower?(String.t(), String.t()) :: boolean()
  def narrower?(new_vis, old_vis) when is_binary(new_vis) and is_binary(old_vis) do
    case {rank(new_vis), rank(old_vis)} do
      {n, o} when is_integer(n) and is_integer(o) -> n < o
      _ -> false
    end
  end

  def narrower?(_, _), do: false

  @doc "Levels that need auth social-graph S2S."
  @spec needs_social?(String.t()) :: boolean()
  def needs_social?(v) when v in ~w(friends friends_plus followers followers_plus), do: true
  def needs_social?(_), do: false
end
