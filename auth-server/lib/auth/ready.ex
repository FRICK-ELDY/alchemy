defmodule Auth.Ready do
  @moduledoc """
  Readiness probes for `GET /ready` (x008b / P2-2).

  Guarantees: Repo can answer `SELECT 1`; Token.Keys exposes a non-empty JWKS.
  Non-guarantees: mailer / external IdP; that verification key rotation paths
  are healthy beyond the published JWKS set.

  Keys failure surface: `Auth.Token.Keys.jwks/0` **raises** when the process or
  `:persistent_term` is missing (fail-closed). World Keys instead exits on
  `GenServer.call` — Ready catches both `rescue` and `:exit` so the probe stays
  symmetrical if Keys implementations converge.
  """

  alias Auth.Repo
  alias Auth.Token.Keys

  @type check_status :: :ok | :error
  @type checks :: %{repo: check_status(), keys: check_status()}

  @spec check() :: {:ok, checks()} | {:error, checks()}
  def check do
    checks = %{
      repo: repo_ok?(),
      keys: keys_ok?()
    }

    if Enum.all?(Map.values(checks), &(&1 == :ok)) do
      {:ok, checks}
    else
      {:error, checks}
    end
  end

  defp repo_ok? do
    case Repo.query("SELECT 1", []) do
      {:ok, _} -> :ok
      {:error, _} -> :error
    end
  rescue
    _ -> :error
  catch
    :exit, _ -> :error
  end

  defp keys_ok? do
    case Keys.jwks() do
      %{"keys" => [_ | _]} -> :ok
      _ -> :error
    end
  rescue
    _ -> :error
  catch
    :exit, _ -> :error
  end
end
