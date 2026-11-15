defmodule Assets.SpawnTicket do
  @moduledoc """
  Issues short-lived SpawnTicket JWTs (RS256) proving Grant-backed placement rights (x00b5).

  Guarantees: `typ=spawn_ticket` / `aud=alchemy-world-spawn`; `room_id` + `blake3s` bound;
  TTL from `:spawn_ticket_ttl_seconds`; max `blake3s` from `:spawn_ticket_max_blake3s`.
  Non-guarantees: world-side consume / participant checks; Origin Cert key reuse;
  Release-tag bulk tickets (future).
  """

  alias Assets.Blobs
  alias Assets.SpawnTicket.Keys

  @typ "spawn_ticket"
  @aud "alchemy-world-spawn"
  @blake3_hex_bytes 64
  @room_id_max_bytes 64

  @doc "Audience claim for SpawnTicket JWTs."
  def audience, do: @aud

  @doc "typ claim value."
  def typ, do: @typ

  @doc """
  Issue a SpawnTicket for `sub` / `room_id` covering `blake3s` (already Grant-checked).

  Does **not** re-check Grant — callers must verify ownership first.
  """
  @spec generate(String.t(), String.t(), [String.t()]) ::
          {:ok, String.t(), pos_integer()} | {:error, term()}
  def generate(sub, room_id, blake3s)
      when is_binary(sub) and is_binary(room_id) and is_list(blake3s) do
    with :ok <- validate_room_id(room_id),
         {:ok, normalized} <- normalize_blake3s(blake3s),
         claims <- build_claims(sub, room_id, normalized),
         {:ok, token} <- sign(claims) do
      ttl = Application.fetch_env!(:assets, :spawn_ticket_ttl_seconds)
      {:ok, token, ttl}
    end
  end

  def generate(_, _, _), do: {:error, :invalid_args}

  @doc """
  Issue after confirming Grant for every blake3 (standalone API / take_out).
  """
  @spec issue_for_grantee(String.t(), String.t(), [String.t()]) ::
          {:ok, String.t(), pos_integer()} | {:error, term()}
  def issue_for_grantee(sub, room_id, blake3s)
      when is_binary(sub) and is_binary(room_id) and is_list(blake3s) do
    with {:ok, normalized} <- normalize_blake3s(blake3s),
         :ok <- assert_all_grants(sub, normalized) do
      generate(sub, room_id, normalized)
    end
  end

  def issue_for_grantee(_, _, _), do: {:error, :invalid_args}

  defp build_claims(sub, room_id, blake3s) do
    now = System.system_time(:second)
    ttl = Application.fetch_env!(:assets, :spawn_ticket_ttl_seconds)

    %{
      "typ" => @typ,
      "sub" => sub,
      "room_id" => room_id,
      "blake3s" => blake3s,
      "jti" => uuid_v4(),
      "iat" => now,
      "nbf" => now,
      "exp" => now + ttl
    }
  end

  defp sign(claims) do
    issuer = Application.fetch_env!(:assets, :spawn_ticket_issuer)

    config =
      Joken.Config.default_claims(skip: [:exp, :iat, :nbf], iss: issuer, aud: @aud)
      |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
      |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("room_id", fn -> nil end, &valid_room_id?/1)
      |> Joken.Config.add_claim("blake3s", fn -> [] end, &valid_blake3s_claim?/1)
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("iat", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("nbf", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("exp", fn -> nil end, &is_integer/1)

    case Joken.generate_and_sign(config, claims, Keys.signer()) do
      {:ok, token, _claims} -> {:ok, token}
      {:error, reason} -> {:error, reason}
    end
  end

  defp assert_all_grants(sub, blake3s) do
    Enum.reduce_while(blake3s, :ok, fn blake3, :ok ->
      case Blobs.assert_grant(sub, blake3) do
        :ok -> {:cont, :ok}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp normalize_blake3s([]) do
    {:error, :blake3s_required}
  end

  defp normalize_blake3s(blake3s) when is_list(blake3s) do
    max = Application.fetch_env!(:assets, :spawn_ticket_max_blake3s)

    if length(blake3s) > max do
      {:error, :blake3s_exceeded}
    else
      collect_normalized_blake3s(blake3s)
    end
  end

  defp collect_normalized_blake3s(blake3s) do
    result =
      Enum.reduce_while(blake3s, {:ok, []}, fn raw, {:ok, acc} ->
        case normalize_blake3(raw) do
          {:ok, b3} -> {:cont, {:ok, [b3 | acc]}}
          {:error, _} = err -> {:halt, err}
        end
      end)

    case result do
      {:ok, list} -> {:ok, list |> Enum.reverse() |> Enum.uniq()}
      {:error, _} = err -> err
    end
  end

  defp normalize_blake3(blake3) when is_binary(blake3) do
    down = String.downcase(blake3)

    if byte_size(down) == @blake3_hex_bytes and Regex.match?(~r/^[0-9a-f]+$/, down) do
      {:ok, down}
    else
      {:error, :invalid_blake3}
    end
  end

  defp normalize_blake3(_), do: {:error, :invalid_blake3}

  defp valid_blake3s_claim?(list) when is_list(list) do
    max = Application.fetch_env!(:assets, :spawn_ticket_max_blake3s)

    list != [] and length(list) <= max and
      Enum.all?(list, &match?({:ok, _}, normalize_blake3(&1)))
  end

  defp valid_blake3s_claim?(_), do: false

  # Mirror World.RoomId / alchemy_protocol::validate_room_id (no shared Elixir dep).
  defp validate_room_id(room_id) do
    if valid_room_id?(room_id), do: :ok, else: {:error, :invalid_room_id}
  end

  defp valid_room_id?(room_id) when is_binary(room_id) do
    room_id != "" and byte_size(room_id) <= @room_id_max_bytes and
      not String.contains?(room_id, "/") and Regex.match?(~r/^[A-Za-z0-9_-]+$/, room_id)
  end

  defp valid_room_id?(_), do: false

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
