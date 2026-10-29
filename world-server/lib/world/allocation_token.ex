defmodule World.AllocationToken do
  @moduledoc """
  Issues short-lived Relay Allocation Tokens (x004e).

  Same RS256 keys / JWKS as Room Tokens; `aud=alchemy-relay`.
  Verified by Relay Agent only (zenohd does not see JWT).
  """

  alias World.Token.Keys

  @roles ~w(host guest)

  @spec generate(String.t(), String.t(), String.t(), String.t(), String.t()) ::
          {:ok, String.t(), non_neg_integer()} | {:error, term()}
  def generate(room_id, role, sub, wire_cap, relay_id)
      when is_binary(room_id) and is_binary(role) and is_binary(sub) and is_binary(wire_cap) and
             is_binary(relay_id) do
    with :ok <- validate_generate_args(role, wire_cap, relay_id) do
      sign_allocation(room_id, role, sub, wire_cap, relay_id)
    end
  end

  def generate(_, _, _, _, _), do: {:error, :invalid_args}

  defp validate_generate_args(role, wire_cap, relay_id)
       when role in @roles and wire_cap != "" and relay_id != "",
       do: :ok

  defp validate_generate_args(_, _, _), do: {:error, :invalid_args}

  defp sign_allocation(room_id, role, sub, wire_cap, relay_id) do
    ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)

    extra = %{
      "sub" => sub,
      "room_id" => room_id,
      "role" => role,
      "wire_cap" => wire_cap,
      "relay_id" => relay_id,
      "jti" => uuid_v4()
    }

    case Joken.generate_and_sign(token_config(), extra, Keys.signer()) do
      {:ok, token, _claims} -> {:ok, token, ttl}
      {:error, reason} -> {:error, reason}
    end
  end

  @spec verify(String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token) when is_binary(token) do
    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- Keys.signer_for_kid(kid),
         {:ok, claims} <- Joken.verify_and_validate(token_config(), token, signer) do
      {:ok, claims}
    else
      {:error, reason} -> {:error, reason}
      other -> {:error, {:unexpected, other}}
    end
  end

  def verify(_), do: {:error, :invalid_token}

  defp token_config do
    ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
    issuer = Application.fetch_env!(:world, :room_token_issuer)
    audience = Application.fetch_env!(:world, :allocation_token_audience)

    Joken.Config.default_claims(default_exp: ttl, iss: issuer, aud: audience)
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("room_id", fn -> nil end, &World.RoomId.valid?/1)
    |> Joken.Config.add_claim("role", fn -> nil end, &(&1 in @roles))
    |> Joken.Config.add_claim("wire_cap", fn -> nil end, &valid_wire_cap?/1)
    |> Joken.Config.add_claim("relay_id", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
  end

  defp valid_wire_cap?(cap) when is_binary(cap) and cap != "" do
    byte_size(cap) <= 64 and Regex.match?(~r/^[A-Za-z0-9_-]+$/, cap)
  end

  defp valid_wire_cap?(_), do: false

  defp peek_kid(token) do
    case Joken.peek_header(token) do
      {:ok, %{"kid" => kid}} when is_binary(kid) and kid != "" ->
        {:ok, %{"kid" => kid}}

      {:ok, _} ->
        {:error, :missing_kid}

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
