defmodule World.Ready do
  @moduledoc """
  Readiness probes for `GET /ready` (x008b / P2-2).

  Guarantees: Repo can answer `SELECT 1`; Token.Keys serves JWKS; seal-hash
  Port worker reports attached Port via non-blocking `ready?/0`.
  Non-guarantees: JWKS freshness vs auth; Port hash throughput; that a
  concurrent reopen will succeed.

  Keys failure surface: `World.Token.Keys.jwks/0` uses `GenServer.call` and
  **exits** when the process is down. Auth Keys raises from `:persistent_term`
  instead — Ready catches both `rescue` and `:exit` so the probe stays
  symmetrical if Keys implementations converge.
  """

  alias World.ManifestSealHash.Worker, as: SealPort
  alias World.Repo
  alias World.Token.Keys

  @type check_status :: :ok | :error
  @type checks :: %{repo: check_status(), keys: check_status(), port: check_status()}

  @spec check() :: {:ok, checks()} | {:error, checks()}
  def check do
    checks = %{
      repo: repo_ok?(),
      keys: keys_ok?(),
      port: port_ok?()
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

  defp port_ok? do
    if SealPort.ready?(), do: :ok, else: :error
  end
end
