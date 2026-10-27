defmodule World.RoomAccess do
  @moduledoc """
  Token-issuance ACL for room visibility (x0034–x0036).

  Owner always allowed. `public` allows any authenticated sub.
  `lan` requires LAN-eligible client path (fail-closed).
  Social levels query auth S2S.

  Non-guarantees: already-issued Room Tokens are not revoked on unfollow /
  unfriend / visibility change; they remain valid until TTL expires.
  """

  alias World.{AuthAccess, Visibility}
  alias WorldWeb.ClientIp

  @spec authorize(Plug.Conn.t(), String.t(), String.t(), String.t()) ::
          :ok
          | {:error, :forbidden_visibility | :lan_required | :auth_unavailable | :not_found}
  def authorize(conn, owner_sub, viewer_sub, visibility)
      when is_binary(owner_sub) and is_binary(viewer_sub) and is_binary(visibility) do
    cond do
      owner_sub == viewer_sub ->
        :ok

      visibility == "public" ->
        :ok

      visibility == "private" ->
        {:error, :forbidden_visibility}

      visibility == "lan" ->
        if ClientIp.lan_eligible?(conn) do
          :ok
        else
          {:error, :lan_required}
        end

      Visibility.needs_social?(visibility) ->
        case AuthAccess.allowed?(owner_sub, viewer_sub, visibility) do
          {:ok, true} -> :ok
          {:ok, false} -> {:error, :forbidden_visibility}
          {:error, _} -> {:error, :auth_unavailable}
        end

      true ->
        {:error, :forbidden_visibility}
    end
  end

  def authorize(_, _, _, _), do: {:error, :forbidden_visibility}
end
