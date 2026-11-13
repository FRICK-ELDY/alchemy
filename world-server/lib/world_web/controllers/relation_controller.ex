defmodule WorldWeb.RelationController do
  @moduledoc """
  auth → world S2S: friendship / follow graph change (x00a0).
  """

  use WorldWeb, :controller

  alias World.Rooms

  @doc """
  Body: `%{"sub_a" => …, "sub_b" => …, "reason" => "unfriend" | "unfollow" | …}`.
  Rotates social-visibility rooms owned by either party.
  """
  def changed(conn, params) do
    sub_a = Map.get(params, "sub_a")
    sub_b = Map.get(params, "sub_b")
    reason = Map.get(params, "reason", "relation")

    with true <- is_binary(sub_a) and sub_a != "",
         true <- is_binary(sub_b) and sub_b != "",
         true <- is_binary(reason) do
      case Rooms.rotate_for_relation_change(sub_a, sub_b) do
        {:ok, rotated} ->
          json(conn, %{ok: true, rotated: rotated, reason: reason})

        {:error, :storage_error} ->
          conn
          |> put_status(:service_unavailable)
          |> json(%{error: "storage_unavailable"})
      end
    else
      _ ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "missing_params"})
    end
  end
end
