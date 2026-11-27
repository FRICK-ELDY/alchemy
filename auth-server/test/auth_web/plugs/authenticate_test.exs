defmodule AuthWeb.Plugs.AuthenticateTest do
  use AuthWeb.ConnCase, async: false

  import ExUnit.CaptureLog

  # Sync: CaptureLog + refute must not pick up failure logs from other async modules.
  test "logs distinct :unknown_kid code for a bearer token with foreign kid" do
    log =
      capture_log(fn ->
        conn =
          build_conn()
          |> put_req_header("authorization", "Bearer " <> jwt_with_kid("foreign-kid"))
          |> get(~p"/api/v1/auth/me")

        assert %{"errors" => %{"detail" => "Unauthorized"}} = json_response(conn, 401)
      end)

    assert log =~ ~s(code: :unknown_kid)
    assert log =~ ~s(event: "auth.authenticate.failure")
    refute log =~ ~s(code: :unexpected_authentication_failure)
  end

  # `Token.verify/1` stops at peek_kid → signer_for_kid; signature/claims need not be valid.
  defp jwt_with_kid(kid) do
    header =
      %{"alg" => "RS256", "kid" => kid}
      |> Jason.encode!()
      |> Base.url_encode64(padding: false)

    payload =
      %{}
      |> Jason.encode!()
      |> Base.url_encode64(padding: false)

    signature = Base.url_encode64("sig", padding: false)
    Enum.join([header, payload, signature], ".")
  end
end
