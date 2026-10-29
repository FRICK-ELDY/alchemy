defmodule World.Token do
  @moduledoc """
  Issues and verifies short-lived Room Tokens (RS256).

  x003e: Intent bind tokens embed `cnf.spk` (base64url Ed25519 session_pub).
  """

  alias World.Token.Keys

  @roles ~w(host guest)
  @session_pub_bytes 32

  @spec generate(String.t(), String.t(), String.t(), binary()) ::
          {:ok, String.t(), non_neg_integer()} | {:error, term()}
  def generate(room_id, role, sub, session_pub)
      when is_binary(room_id) and is_binary(role) and is_binary(sub) and
             is_binary(session_pub) and byte_size(session_pub) == @session_pub_bytes do
    if role in @roles do
      ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
      jti = uuid_v4()
      spk = Base.url_encode64(session_pub, padding: false)

      extra = %{
        "sub" => sub,
        "room_id" => room_id,
        "role" => role,
        "jti" => jti,
        "cnf" => %{"spk" => spk}
      }

      case Joken.generate_and_sign(token_config(), extra, Keys.signer()) do
        {:ok, token, _claims} -> {:ok, token, ttl}
        {:error, reason} -> {:error, reason}
      end
    else
      {:error, :invalid_role}
    end
  end

  def generate(_, _, _, _), do: {:error, :invalid_args}

  @doc "Decode base64url session_pub from API body into 32 raw bytes (x003e)."
  @spec decode_session_pub(String.t()) :: {:ok, binary()} | {:error, :invalid_session_pub}
  def decode_session_pub(encoded) when is_binary(encoded) do
    case Base.url_decode64(encoded, padding: false) do
      {:ok, bin} when byte_size(bin) == @session_pub_bytes -> {:ok, bin}
      _ -> {:error, :invalid_session_pub}
    end
  end

  def decode_session_pub(_), do: {:error, :invalid_session_pub}

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
    audience = Application.fetch_env!(:world, :room_token_audience)

    Joken.Config.default_claims(default_exp: ttl, iss: issuer, aud: audience)
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("room_id", fn -> nil end, &valid_room_id?/1)
    |> Joken.Config.add_claim("role", fn -> nil end, &(&1 in @roles))
    |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("cnf", fn -> nil end, &valid_cnf?/1)
  end

  defp valid_cnf?(%{"spk" => spk}) when is_binary(spk) do
    match?({:ok, _}, decode_session_pub(spk))
  end

  defp valid_cnf?(_), do: false

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

  defp valid_room_id?(value), do: World.RoomId.valid?(value)

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
