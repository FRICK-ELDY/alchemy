defmodule AuthWeb.SocialController do
  use AuthWeb, :controller

  alias Auth.Social

  def follow(conn, %{"user_id" => user_id}) do
    case Social.follow(conn.assigns.current_user_id, user_id) do
      {:ok, _} ->
        json(conn, %{ok: true, following: user_id})

      {:error, :self} ->
        unprocessable(conn, "cannot_follow_self")

      {:error, :not_found} ->
        not_found(conn, "user_not_found")

      {:error, :already_following} ->
        json(conn, %{ok: true, following: user_id, already: true})

      {:error, _} ->
        unprocessable(conn, "follow_failed")
    end
  end

  def unfollow(conn, %{"user_id" => user_id}) do
    case Social.unfollow(conn.assigns.current_user_id, user_id) do
      :ok ->
        json(conn, %{ok: true})

      {:error, :self} ->
        unprocessable(conn, "cannot_unfollow_self")

      {:error, :not_found} ->
        not_found(conn, "not_following")

      {:error, _} ->
        unprocessable(conn, "unfollow_failed")
    end
  end

  def request_friend(conn, %{"user_id" => user_id}) do
    case Social.request_friend(conn.assigns.current_user_id, user_id) do
      {:ok, row} ->
        conn
        |> put_status(:created)
        |> json(%{ok: true, state: row.state, user_id: user_id})

      {:error, :not_mutual} ->
        unprocessable(conn, "not_mutual")

      {:error, :already_friends} ->
        unprocessable(conn, "already_friends")

      {:error, :already_pending} ->
        unprocessable(conn, "already_pending")

      {:error, :self} ->
        unprocessable(conn, "cannot_friend_self")

      {:error, :not_found} ->
        not_found(conn, "user_not_found")

      {:error, _} ->
        unprocessable(conn, "friend_request_failed")
    end
  end

  def accept_friend(conn, %{"user_id" => user_id}) do
    case Social.accept_friend(conn.assigns.current_user_id, user_id) do
      {:ok, row} ->
        json(conn, %{ok: true, state: row.state, user_id: user_id})

      {:error, :forbidden} ->
        forbidden(conn, "not_addressee")

      {:error, :not_pending} ->
        unprocessable(conn, "not_pending")

      {:error, :not_found} ->
        not_found(conn, "request_not_found")

      {:error, _} ->
        unprocessable(conn, "accept_failed")
    end
  end

  def reject_friend(conn, %{"user_id" => user_id}) do
    case Social.reject_friend(conn.assigns.current_user_id, user_id) do
      :ok ->
        json(conn, %{ok: true})

      {:error, :already_friends} ->
        unprocessable(conn, "already_friends")

      {:error, :not_found} ->
        not_found(conn, "request_not_found")

      {:error, _} ->
        unprocessable(conn, "reject_failed")
    end
  end


  def unfriend(conn, %{"user_id" => user_id}) do
    case Social.unfriend(conn.assigns.current_user_id, user_id) do
      :ok ->
        json(conn, %{ok: true})

      {:error, :not_found} ->
        not_found(conn, "not_friends")

      {:error, _} ->
        unprocessable(conn, "unfriend_failed")
    end
  end

  defp unprocessable(conn, code) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: code})
  end

  defp not_found(conn, code) do
    conn
    |> put_status(:not_found)
    |> json(%{error: code})
  end

  defp forbidden(conn, code) do
    conn
    |> put_status(:forbidden)
    |> json(%{error: code})
  end
end
