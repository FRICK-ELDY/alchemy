defmodule Auth.Social do
  @moduledoc """
  Follow + Friendship operations (x0033).

  Guarantees:
  - Directed follow / unfollow
  - Friend request only when mutual follow
  - Unfollow clears friendship (pending or accepted) for that pair (same DB transaction)

  Non-guarantees: Block, Group, push notifications, JWT claims for the graph;
  Room Token revocation on unfollow (world issues tokens; TTL-only until later).
  """

  require Logger

  alias Auth.Accounts.User
  alias Auth.Repo
  alias Auth.Social.{Follow, Friendship}

  @type user_id :: String.t() | Ecto.UUID.t()

  @spec follow(user_id(), user_id()) ::
          {:ok, Follow.t()} | {:error, :self | :not_found | :already_following | term()}
  def follow(follower_id, followee_id) do
    with {:ok, follower_id} <- cast_id(follower_id),
         {:ok, followee_id} <- cast_id(followee_id),
         :ok <- reject_self(follower_id, followee_id),
         :ok <- ensure_user(follower_id),
         :ok <- ensure_user(followee_id) do
      case Follow.create(%{follower_id: follower_id, followee_id: followee_id}) do
        {:ok, edge} ->
          {:ok, edge}

        {:error, error} ->
          if unique_violation?(error), do: {:error, :already_following}, else: {:error, error}
      end
    end
  end

  @spec unfollow(user_id(), user_id()) ::
          :ok | {:error, :self | :not_found | term()}
  def unfollow(follower_id, followee_id) do
    with {:ok, follower_id} <- cast_id(follower_id),
         {:ok, followee_id} <- cast_id(followee_id),
         :ok <- reject_self(follower_id, followee_id),
         {:ok, %Follow{} = edge} <- fetch_follow(follower_id, followee_id) do
      case Repo.transaction(fn ->
             with :ok <- destroy_follow(edge),
                  :ok <- clear_friendship(follower_id, followee_id) do
               :ok
             else
               {:error, reason} -> Repo.rollback(reason)
             end
           end) do
        {:ok, :ok} -> :ok
        {:error, reason} -> {:error, reason}
      end
    end
  end

  @spec following?(user_id(), user_id()) :: boolean()
  def following?(follower_id, followee_id) do
    match?({:ok, %Follow{}}, fetch_follow(follower_id, followee_id))
  end

  @spec mutual?(user_id(), user_id()) :: boolean()
  def mutual?(a, b), do: following?(a, b) and following?(b, a)

  @spec request_friend(user_id(), user_id()) ::
          {:ok, Friendship.t()}
          | {:error,
             :self | :not_mutual | :already_friends | :already_pending | :not_found | term()}
  def request_friend(requester_id, addressee_id) do
    with {:ok, requester_id} <- cast_id(requester_id),
         {:ok, addressee_id} <- cast_id(addressee_id),
         :ok <- reject_self(requester_id, addressee_id),
         :ok <- ensure_user(requester_id),
         :ok <- ensure_user(addressee_id),
         true <- mutual?(requester_id, addressee_id) || {:error, :not_mutual} do
      {lo, hi} = pair(requester_id, addressee_id)

      case Friendship.get_pair(lo, hi) do
        {:ok, %Friendship{state: :accepted}} ->
          {:error, :already_friends}

        {:ok, %Friendship{state: :pending}} ->
          {:error, :already_pending}

        {:ok, nil} ->
          create_pending(lo, hi, requester_id)

        {:error, error} ->
          if not_found?(error) do
            create_pending(lo, hi, requester_id)
          else
            {:error, error}
          end
      end
    else
      {:error, _} = err -> err
      false -> {:error, :not_mutual}
    end
  end

  @spec accept_friend(user_id(), user_id()) ::
          {:ok, Friendship.t()} | {:error, :not_found | :forbidden | :not_pending | term()}
  def accept_friend(accepter_id, requester_id) do
    with {:ok, accepter_id} <- cast_id(accepter_id),
         {:ok, requester_id} <- cast_id(requester_id),
         :ok <- reject_self(accepter_id, requester_id),
         {lo, hi} <- pair(accepter_id, requester_id),
         {:ok, %Friendship{} = row} <- fetch_pair(lo, hi) do
      cond do
        row.state != :pending ->
          {:error, :not_pending}

        row.requester_id == accepter_id ->
          {:error, :forbidden}

        row.requester_id != requester_id ->
          {:error, :forbidden}

        true ->
          Friendship.accept(row)
      end
    else
      {:error, _} = err -> err
      _ -> {:error, :not_found}
    end
  end

  @spec reject_friend(user_id(), user_id()) ::
          :ok | {:error, :not_found | :already_friends | term()}
  def reject_friend(actor_id, other_id) do
    with {:ok, actor_id} <- cast_id(actor_id),
         {:ok, other_id} <- cast_id(other_id),
         {lo, hi} <- pair(actor_id, other_id),
         {:ok, row} <- fetch_pair(lo, hi) do
      case row do
        %Friendship{state: :pending} = pending ->
          destroy_friendship(pending)

        %Friendship{state: :accepted} ->
          {:error, :already_friends}
      end
    end
  end


  @spec unfriend(user_id(), user_id()) :: :ok | {:error, :not_found | term()}
  def unfriend(a, b) do
    with {:ok, a} <- cast_id(a),
         {:ok, b} <- cast_id(b),
         {lo, hi} <- pair(a, b),
         {:ok, %Friendship{state: :accepted} = row} <- fetch_pair(lo, hi) do
      destroy_friendship(row)
    else
      {:error, _} = err -> err
      _ -> {:error, :not_found}
    end
  end

  @spec friends?(user_id(), user_id()) :: boolean()
  def friends?(a, b) do
    with {:ok, a} <- cast_id(a),
         {:ok, b} <- cast_id(b),
         {lo, hi} <- pair(a, b),
         {:ok, %Friendship{state: :accepted}} <- fetch_pair(lo, hi) do
      true
    else
      _ -> false
    end
  end

  @doc "Accepted friends of `user_id` (other party ids)."
  @spec friend_ids(user_id()) :: [Ecto.UUID.t()]
  def friend_ids(user_id) do
    case cast_id(user_id) do
      {:ok, user_id} ->
        case Friendship.list_accepted_for(user_id) do
          {:ok, rows} -> Enum.map(rows, &other_of(&1, user_id))
          _ -> []
        end

      _ ->
        []
    end
  end

  @doc "Friends ∪ friends-of-friends (x0035). Two list fetches + set intersection."
  @spec friends_plus?(user_id(), user_id()) :: boolean()
  def friends_plus?(owner_id, viewer_id) do
    with {:ok, owner_id} <- cast_id(owner_id),
         {:ok, viewer_id} <- cast_id(viewer_id) do
      owner_friends = MapSet.new(friend_ids(owner_id))

      cond do
        MapSet.member?(owner_friends, viewer_id) ->
          true

        true ->
          viewer_friends = MapSet.new(friend_ids(viewer_id))
          not MapSet.disjoint?(owner_friends, viewer_friends)
      end
    else
      _ -> false
    end
  end

  @doc "Viewer follows owner (1-hop inbound)."
  @spec follower?(user_id(), user_id()) :: boolean()
  def follower?(owner_id, viewer_id), do: following?(viewer_id, owner_id)

  @doc "Followers ∪ people who follow those followers (x0035). Two list fetches + intersection."
  @spec followers_plus?(user_id(), user_id()) :: boolean()
  def followers_plus?(owner_id, viewer_id) do
    with {:ok, owner_id} <- cast_id(owner_id),
         {:ok, viewer_id} <- cast_id(viewer_id) do
      if following?(viewer_id, owner_id) do
        true
      else
        with {:ok, follower_edges} <- Follow.list_followers(owner_id),
             {:ok, following_edges} <- Follow.list_following(viewer_id) do
          followers = MapSet.new(Enum.map(follower_edges, & &1.follower_id))
          following = MapSet.new(Enum.map(following_edges, & &1.followee_id))
          not MapSet.disjoint?(followers, following)
        else
          _ -> false
        end
      end
    else
      _ -> false
    end
  end

  defp create_pending(lo, hi, requester_id) do
    case Friendship.create(%{
           sub_lo: lo,
           sub_hi: hi,
           requester_id: requester_id,
           state: :pending
         }) do
      {:ok, row} ->
        {:ok, row}

      {:error, error} ->
        if unique_violation?(error) do
          case Friendship.get_pair(lo, hi) do
            {:ok, %Friendship{state: :accepted}} -> {:error, :already_friends}
            {:ok, %Friendship{state: :pending}} -> {:error, :already_pending}
            _ -> {:error, :already_pending}
          end
        else
          {:error, error}
        end
    end
  end

  defp clear_friendship(a, b) do
    {lo, hi} = pair(a, b)

    case fetch_pair(lo, hi) do
      {:ok, %Friendship{} = row} -> destroy_friendship(row)
      {:error, :not_found} -> :ok
      {:error, reason} -> {:error, reason}
    end
  end

  defp fetch_follow(follower_id, followee_id) do
    with {:ok, follower_id} <- cast_id(follower_id),
         {:ok, followee_id} <- cast_id(followee_id) do
      case Follow.get_edge(follower_id, followee_id) do
        {:ok, %Follow{} = edge} -> {:ok, edge}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> if not_found?(error), do: {:error, :not_found}, else: {:error, error}
      end
    end
  end

  defp destroy_follow(edge) do
    case Follow.destroy(edge) do
      :ok -> :ok
      {:ok, _} -> :ok
      {:error, error} -> {:error, error}
    end
  end

  defp destroy_friendship(row) do
    case Friendship.destroy(row) do
      :ok -> :ok
      {:ok, _} -> :ok
      {:error, error} -> {:error, error}
    end
  end

  defp fetch_pair(lo, hi) do
    case Friendship.get_pair(lo, hi) do
      {:ok, %Friendship{} = row} -> {:ok, row}
      {:ok, nil} -> {:error, :not_found}
      {:error, error} -> if not_found?(error), do: {:error, :not_found}, else: {:error, error}
    end
  end

  defp pair(a, b) when a <= b, do: {a, b}
  defp pair(a, b), do: {b, a}

  defp other_of(%Friendship{sub_lo: lo, sub_hi: hi}, id) when id == lo, do: hi
  defp other_of(%Friendship{sub_lo: lo, sub_hi: hi}, id) when id == hi, do: lo

  defp reject_self(a, a), do: {:error, :self}
  defp reject_self(_, _), do: :ok

  defp ensure_user(id) do
    case Ash.get(User, id) do
      {:ok, %User{}} -> :ok
      {:ok, nil} -> {:error, :not_found}
      {:error, error} -> if not_found?(error), do: {:error, :not_found}, else: {:error, error}
    end
  end

  defp cast_id(id) when is_binary(id) do
    case Ecto.UUID.cast(id) do
      {:ok, uuid} -> {:ok, uuid}
      :error -> {:error, :not_found}
    end
  end

  defp cast_id(id), do: {:error, {:invalid_id, id}}

  defp not_found?(error) do
    leaves =
      case error do
        %{errors: errors} when is_list(errors) -> errors
        other -> [other]
      end

    Enum.any?(leaves, fn
      %Ash.Error.Query.NotFound{} -> true
      _ -> Exception.message(error) =~ "record not found"
    end)
  end

  defp unique_violation?(error) do
    message = Exception.message(error)

    String.contains?(message, "unique") or
      String.contains?(message, "already been taken") or
      String.contains?(message, "UNIQUE")
  end
end
