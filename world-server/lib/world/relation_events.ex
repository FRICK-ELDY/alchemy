defmodule World.RelationEvents do
  @moduledoc """
  Idempotent record of auth `relation_changed` event ids (x00d7).

  Guarantees: `event_id` is recorded **only after** a successful rotate batch so auth
  outbox retry can still close incomplete work; duplicate recorded ids return
  `:already_applied`.
  Non-guarantees: cross-node distributed lock beyond SQLite uniqueness; partial
  rotate mid-batch is repaired by retry (may over-rotate already-bumped rooms).
  """

  @spec applied?(String.t() | nil) :: boolean() | {:error, :storage_error}
  def applied?(nil), do: false
  def applied?(""), do: false

  def applied?(event_id) when is_binary(event_id) do
    case World.Repo.query(
           "SELECT 1 FROM relation_events WHERE event_id = ? LIMIT 1",
           [event_id]
         ) do
      {:ok, %{num_rows: n}} when is_integer(n) and n > 0 -> true
      {:ok, _} -> false
      {:error, _} -> {:error, :storage_error}
    end
  rescue
    _ -> {:error, :storage_error}
  end

  def applied?(_), do: false

  @doc """
  Record a successfully applied `event_id`. Call only after rotate succeeds.
  """
  @spec record(String.t() | nil) :: :ok | :already_applied | {:error, :storage_error}
  def record(nil), do: :ok
  def record(""), do: :ok

  def record(event_id) when is_binary(event_id) do
    now = DateTime.utc_now() |> DateTime.to_iso8601()

    case World.Repo.query(
           "INSERT INTO relation_events (event_id, consumed_at) VALUES (?, ?)",
           [event_id, now]
         ) do
      {:ok, %{num_rows: 1}} ->
        :ok

      {:error, %Exqlite.Error{message: message}} ->
        if unique_violation?(message) do
          :already_applied
        else
          {:error, :storage_error}
        end

      {:error, _} ->
        {:error, :storage_error}
    end
  rescue
    error ->
      if unique_violation?(Exception.message(error)) do
        :already_applied
      else
        {:error, :storage_error}
      end
  end

  def record(_), do: :ok

  defp unique_violation?(message) when is_binary(message) do
    String.contains?(message, "UNIQUE") or String.contains?(message, "unique")
  end

  defp unique_violation?(_), do: false
end
