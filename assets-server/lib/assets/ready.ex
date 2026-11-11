defmodule Assets.Ready do
  @moduledoc """
  Readiness probes for `GET /ready` (x008b / P2-2).

  Guarantees: Repo can answer `SELECT 1`; BLAKE3 Port worker reports attached
  Port via non-blocking `ready?/0` (no GenServer mailbox wait).
  Non-guarantees: CAS backend reachability; Port hash throughput; auth JWKS
  (assets verifies via shared client, not a local Keys process).
  """

  alias Assets.Blake3.Worker, as: Blake3Port
  alias Assets.Repo

  @type check_status :: :ok | :error
  @type checks :: %{repo: check_status(), port: check_status()}

  @spec check() :: {:ok, checks()} | {:error, checks()}
  def check do
    checks = %{
      repo: repo_ok?(),
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

  defp port_ok? do
    if Blake3Port.ready?(), do: :ok, else: :error
  end
end
