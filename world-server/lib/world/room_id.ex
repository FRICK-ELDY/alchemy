defmodule World.RoomId do
  @moduledoc """
  room_id validation matching `alchemy_protocol::validate_room_id` (x002e).

  Shared vectors: see `protocol` crate test `validate_room_id_shared_vectors`
  and `World.RoomIdTest`.

  Charset `[A-Za-z0-9_-]+` applies to HostAttestation / Token / Rooms (PIN bump).
  Existing rooms use `local` or hex `random_room_id/0` — no pre-charset legacy IDs.
  """

  @max_bytes 64

  @doc "Max byte length (protocol `ROOM_ID_MAX_BYTES`)."
  @spec max_bytes() :: pos_integer()
  def max_bytes, do: @max_bytes

  @spec validate(term()) :: :ok | {:error, :invalid_room_id}
  def validate(room_id) when is_binary(room_id) do
    cond do
      room_id == "" ->
        {:error, :invalid_room_id}

      # `/` is also outside the charset; checked first to mirror protocol ContainsSlash
      # (Zenoh key-segment break) before the generic InvalidCharset path.
      String.contains?(room_id, "/") ->
        {:error, :invalid_room_id}

      byte_size(room_id) > @max_bytes ->
        {:error, :invalid_room_id}

      not Regex.match?(~r/^[A-Za-z0-9_-]+$/, room_id) ->
        {:error, :invalid_room_id}

      true ->
        :ok
    end
  end

  def validate(_), do: {:error, :invalid_room_id}

  @spec valid?(term()) :: boolean()
  def valid?(value), do: validate(value) == :ok
end
