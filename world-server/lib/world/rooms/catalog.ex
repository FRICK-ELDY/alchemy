defmodule World.Rooms.Catalog do
  @moduledoc """
  Room catalog listing under RoomAccess (x007c; x00d6 split from `World.Rooms`).

  Guarantees: summaries never contain `room_key`, `wire_cap`, or scene; ACL via `RoomAccess`;
  active state is derived from an unexpired host lease.

  Non-guarantees: friendship-graph-aware scan windows; auth of the viewer (caller supplies `viewer_sub`).

  Callers should use the `World.Rooms` facade; this module is the implementation unit.
  """

  import World.Rooms.Internal, only: [lease_valid?: 1, storage_error: 2]

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  @doc """
  Catalog summaries visible to `viewer_sub` under RoomAccess (x007c).

  Does **not** include `room_key`, Token, `wire_cap`, or scene.
  `role_hint` is `owner` | `joinable` only (invisible rooms are omitted).

  Guarantees: owned rooms (up to owned limit) and public candidates are not
  dropped by a global top-N of all rooms before ACL.

  Non-guarantees: social/lan candidates are still “newest N by visibility class”
  then ACL-filtered — a friend’s rarely-updated room may fall outside the social
  scan window (friendship-graph-aware scan is out of scope).
  """
  @type catalog_summary :: %{
          room_id: room_id(),
          visibility: String.t(),
          owner_sub: owner_sub(),
          role_hint: String.t(),
          state: String.t(),
          active_host_sub: owner_sub() | nil
        }

  # Per-bucket caps: owned + public + social(+lan) candidates, then ACL filter.
  # Avoids "global ORDER BY LIMIT then filter" dropping older visible private/friends rooms.
  @catalog_owned_limit 200
  @catalog_public_limit 200
  @catalog_social_limit 200
  @catalog_result_limit 200

  @spec list_visible_summaries(Plug.Conn.t(), owner_sub(), keyword()) ::
          {:ok, [catalog_summary()]} | {:error, storage_error()}
  def list_visible_summaries(conn, viewer_sub, opts \\ [])

  def list_visible_summaries(conn, viewer_sub, opts)
      when is_binary(viewer_sub) and viewer_sub != "" and is_list(opts) do
    mine_only? = Keyword.get(opts, :mine_only, false) == true
    include_lan? = not mine_only? and WorldWeb.ClientIp.lan_eligible?(conn)

    case list_catalog_rows(viewer_sub, mine_only?, include_lan?) do
      {:ok, rows} ->
        summaries =
          rows
          |> Enum.filter(fn row -> catalog_visible?(conn, viewer_sub, row) end)
          |> Enum.map(fn row -> catalog_summary(viewer_sub, row) end)
          |> Enum.take(@catalog_result_limit)

        {:ok, summaries}

      {:error, :storage_error} = err ->
        err
    end
  end

  def list_visible_summaries(_, _, _), do: {:error, :storage_error}

  defp list_catalog_rows(viewer_sub, true, _include_lan?) do
    case World.Repo.query(
           """
           SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at
           FROM rooms
           WHERE owner_sub = ?
           ORDER BY updated_at DESC
           LIMIT ?
           """,
           [viewer_sub, @catalog_owned_limit]
         ) do
      {:ok, %{rows: rows}} -> {:ok, Enum.map(rows, &catalog_row/1)}
      {:error, error} -> storage_error(error, :list_catalog_rows)
    end
  rescue
    e in [RuntimeError, ArgumentError] ->
      storage_error(e, :list_catalog_rows)
  end

  defp list_catalog_rows(viewer_sub, false, include_lan?) do
    social_vis =
      if include_lan? do
        ["friends", "friends_plus", "followers", "followers_plus", "lan"]
      else
        ["friends", "friends_plus", "followers", "followers_plus"]
      end

    # Union owned ∪ public ∪ social/lan candidates (dedupe by room_id), newest first.
    # x0080: non-mine catalog is live sessions only (active + unexpired lease).
    # Visibility ACL (incl. social S2S) still applied in catalog_visible?/3.
    placeholders = Enum.map_join(social_vis, ",", fn _ -> "?" end)
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    sql = """
    SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at FROM (
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 0 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE owner_sub = ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
      UNION
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 1 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE visibility = 'public' AND owner_sub != ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
      UNION
      SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at, 2 AS pri FROM (
        SELECT room_id, owner_sub, visibility, state, active_host_sub, host_lease_expires_at, updated_at
        FROM rooms
        WHERE visibility IN (#{placeholders}) AND owner_sub != ?
          AND state = 'active'
          AND host_lease_expires_at IS NOT NULL
          AND host_lease_expires_at > ?
        ORDER BY updated_at DESC
        LIMIT ?
      )
    )
    ORDER BY pri ASC, updated_at DESC
    """

    params =
      List.flatten([
        viewer_sub,
        now,
        @catalog_owned_limit,
        viewer_sub,
        now,
        @catalog_public_limit,
        social_vis,
        viewer_sub,
        now,
        @catalog_social_limit
      ])

    case World.Repo.query(sql, params) do
      {:ok, %{rows: rows}} -> {:ok, Enum.map(rows, &catalog_row/1)}
      {:error, error} -> storage_error(error, :list_catalog_rows)
    end
  rescue
    e in [RuntimeError, ArgumentError] ->
      storage_error(e, :list_catalog_rows)
  end

  defp catalog_row([
         room_id,
         owner_sub,
         visibility,
         state,
         active_host_sub,
         host_lease_expires_at
       ])
       when is_binary(room_id) and is_binary(owner_sub) and is_binary(visibility) and
              is_binary(state) do
    %{
      room_id: room_id,
      owner_sub: owner_sub,
      visibility: visibility,
      state: state,
      active_host_sub: active_host_sub,
      host_lease_expires_at: host_lease_expires_at
    }
  end

  defp catalog_visible?(conn, viewer_sub, %{
         owner_sub: owner_sub,
         visibility: visibility
       }) do
    case World.RoomAccess.authorize(conn, owner_sub, viewer_sub, visibility) do
      :ok -> true
      # Fail closed for catalog: social outage / forbidden / lan → omit row.
      _ -> false
    end
  end

  defp catalog_summary(viewer_sub, row) do
    role_hint = if row.owner_sub == viewer_sub, do: "owner", else: "joinable"

    %{
      room_id: row.room_id,
      visibility: row.visibility,
      owner_sub: row.owner_sub,
      role_hint: role_hint,
      state: effective_catalog_state(row),
      active_host_sub: effective_catalog_active_host(row)
    }
  end

  defp effective_catalog_state(%{state: "active"} = row) do
    if lease_valid?(row.host_lease_expires_at), do: "active", else: "dormant"
  end

  defp effective_catalog_state(%{state: state}) when is_binary(state), do: state
  defp effective_catalog_state(_), do: "dormant"

  defp effective_catalog_active_host(%{state: "active"} = row) do
    if lease_valid?(row.host_lease_expires_at), do: row.active_host_sub, else: nil
  end

  defp effective_catalog_active_host(_), do: nil
end
