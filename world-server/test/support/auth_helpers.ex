defmodule World.TestAuthHelpers do
  @moduledoc """
  Installs a local RSA keypair as fake auth-server JWKS for controller tests.
  x0081: also helpers for host HTTP PoP (`cnf.spk` Ed25519).
  """

  alias World.Auth.Jwks
  alias World.HttpPop
  alias WorldWeb.Endpoint

  @issuer "alchemy-auth"
  @audience "alchemy-platform"

  def install_test_auth! do
    jwk = JOSE.JWK.generate_key({:rsa, 2048})
    public_jwk = JOSE.JWK.to_public(jwk)
    kid = thumbprint_kid(public_jwk)
    {_fields, jwk_map} = JOSE.JWK.to_map(public_jwk)
    {_type, private_pem} = JOSE.JWK.to_pem(jwk)

    public_key_map =
      jwk_map
      |> Map.put("kid", kid)
      |> Map.put("use", "sig")
      |> Map.put("alg", "RS256")

    jwks = %{"keys" => [public_key_map]}

    verify_signer =
      Joken.Signer.create("RS256", %{
        "kty" => "RSA",
        "n" => public_key_map["n"],
        "e" => public_key_map["e"],
        "kid" => kid
      })

    sign_signer = Joken.Signer.create("RS256", %{"pem" => private_pem}, %{"kid" => kid})
    Jwks.install_test_jwks!(jwks, %{kid => verify_signer})

    %{kid: kid, signer: sign_signer}
  end

  def auth_token(signer, sub \\ "11111111-1111-1111-1111-111111111111") do
    claims = %{
      "sub" => sub,
      "status" => "active",
      "jti" => uuid_v4(),
      "iss" => @issuer,
      "aud" => @audience
    }

    config =
      Joken.Config.default_claims(default_exp: 900, iss: @issuer, aud: @audience)
      |> Joken.Config.add_claim("sub", fn -> sub end, &is_binary/1)
      |> Joken.Config.add_claim("status", fn -> "active" end, &(&1 == "active"))
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

    {:ok, token, _} = Joken.generate_and_sign(config, claims, signer)
    token
  end

  def put_auth(conn, signer, sub \\ "11111111-1111-1111-1111-111111111111") do
    Plug.Conn.put_req_header(conn, "authorization", "Bearer " <> auth_token(signer, sub))
  end

  @doc "Generate Ed25519 keypair for Room Token `session_pub` / HTTP PoP (x0081)."
  def session_keypair do
    {pub, priv} = :crypto.generate_key(:eddsa, :ed25519)
    {pub, priv}
  end

  @doc "base64url session_pub for `POST …/tokens`."
  def session_pub_b64(pub) when is_binary(pub) do
    Base.url_encode64(pub, padding: false)
  end

  @doc """
  Attach host Room Token Bearer + PoP headers.

  `body` must be the exact raw HTTP body bytes that will be sent (\"\" for GET/DELETE).
  """
  def put_host_pop(conn, token, priv, method, path, body \\ "")
      when is_binary(token) and is_binary(priv) and is_binary(method) and is_binary(path) and
             is_binary(body) do
    ts = System.system_time(:second)
    msg = HttpPop.signing_bytes(method, path, ts, body)
    sig = :crypto.sign(:eddsa, :none, msg, [priv, :ed25519])

    conn
    |> Plug.Conn.put_req_header("authorization", "Bearer " <> token)
    |> Plug.Conn.put_req_header("x-alchemy-pop-ts", Integer.to_string(ts))
    |> Plug.Conn.put_req_header("x-alchemy-pop-sig", Base.url_encode64(sig, padding: false))
  end

  @doc "JSON POST with host Token + PoP (body signed as Jason.encode!)."
  def post_host_json(conn, token, priv, path, body_map) when is_map(body_map) do
    raw = Jason.encode!(body_map)

    conn
    |> put_host_pop(token, priv, "POST", path, raw)
    |> Plug.Conn.put_req_header("content-type", "application/json")
    |> Phoenix.ConnTest.dispatch(Endpoint, :post, path, raw)
  end

  @doc "GET with host Token + PoP."
  def get_host(conn, token, priv, path) do
    conn
    |> put_host_pop(token, priv, "GET", path, "")
    |> Phoenix.ConnTest.dispatch(Endpoint, :get, path)
  end

  @doc "DELETE with host Token + PoP (empty body)."
  def delete_host(conn, token, priv, path) do
    conn
    |> put_host_pop(token, priv, "DELETE", path, "")
    |> Phoenix.ConnTest.dispatch(Endpoint, :delete, path)
  end

  @doc "JSON DELETE with host Token + PoP (body signed as Jason.encode! · x0082)."
  def delete_host_json(conn, token, priv, path, body_map) when is_map(body_map) do
    raw = Jason.encode!(body_map)

    conn
    |> put_host_pop(token, priv, "DELETE", path, raw)
    |> Plug.Conn.put_req_header("content-type", "application/json")
    |> Phoenix.ConnTest.dispatch(Endpoint, :delete, path, raw)
  end

  defp thumbprint_kid(jwk) do
    case JOSE.JWK.thumbprint(jwk) do
      {:kid, kid} -> kid
      kid when is_binary(kid) -> kid
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
