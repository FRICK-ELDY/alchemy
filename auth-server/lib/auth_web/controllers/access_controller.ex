defmodule AuthWeb.AccessController do
  use AuthWeb, :controller

  alias Auth.Access

  def show(conn, params) do
    owner = Map.get(params, "owner_sub")
    viewer = Map.get(params, "viewer_sub")
    visibility = Map.get(params, "visibility")

    with true <- is_binary(owner) and owner != "",
         true <- is_binary(viewer) and viewer != "",
         {:ok, vis} <- Access.parse_visibility(visibility) do
      json(conn, %{
        allowed: Access.allowed?(owner, viewer, vis),
        owner_sub: owner,
        viewer_sub: viewer,
        visibility: Atom.to_string(vis)
      })
    else
      {:error, :invalid_visibility} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "invalid_visibility"})

      _ ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "missing_params"})
    end
  end
end
