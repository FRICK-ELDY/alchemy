defmodule WorldWeb.RelationController do
  @moduledoc """
  auth → world S2S: friendship / follow graph change (x00a0 / **x00d7**).
  """

  use WorldWeb, :controller

  alias World.Rooms

  @doc """
  Body: `%{"sub_a" => …, "sub_b" => …, "reason" => …}` plus optional
  `"event_id"` and `"affected_owner_subs"` (x00d7).
  Rotates social-visibility rooms owned by the affected owner set.
  """
  def changed(conn, params) do
    sub_a = Map.get(params, "sub_a")
    sub_b = Map.get(params, "sub_b")
    reason = Map.get(params, "reason", "relation")

    if valid_relation_params?(sub_a, sub_b, reason) do
      opts = relation_rotate_opts(params)
      render_relation_rotate(conn, Rooms.rotate_for_relation_change(sub_a, sub_b, opts), reason)
    else
      conn
      |> put_status(:unprocessable_entity)
      |> json(%{error: "missing_params"})
    end
  end

  defp valid_relation_params?(sub_a, sub_b, reason) do
    is_binary(sub_a) and sub_a != "" and is_binary(sub_b) and sub_b != "" and is_binary(reason)
  end

  defp relation_rotate_opts(params) do
    owners = normalize_owners(Map.get(params, "affected_owner_subs"))
    opts = [event_id: Map.get(params, "event_id")]

    if owners == [] do
      opts
    else
      Keyword.put(opts, :affected_owner_subs, owners)
    end
  end

  defp render_relation_rotate(conn, {:ok, :already_applied}, reason) do
    json(conn, %{ok: true, already_applied: true, rotated: [], reason: reason})
  end

  defp render_relation_rotate(conn, {:ok, rotated}, reason) do
    json(conn, %{ok: true, rotated: rotated, reason: reason})
  end

  defp render_relation_rotate(conn, {:error, :storage_error}, _reason) do
    conn
    |> put_status(:service_unavailable)
    |> json(%{error: "storage_unavailable"})
  end

  defp normalize_owners(list) when is_list(list) do
    list
    |> Enum.filter(&(is_binary(&1) and &1 != ""))
    |> Enum.uniq()
  end

  defp normalize_owners(_), do: []
end
