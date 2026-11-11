defmodule Assets.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    if Application.fetch_env!(:assets, :cas_backend) == :local do
      File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    end

    Assets.Metrics.ensure_table()
    Assets.PlaceRead.ensure_verify_cache!()

    children =
      [
        AssetsWeb.Telemetry,
        Assets.Repo,
        {Phoenix.PubSub, name: Assets.PubSub},
        Assets.Auth.Jwks,
        Assets.World.Jwks,
        Assets.PlaceHashCache,
        # x0075 / x008d: blake3-hash Port pool (PartitionSupervisor; no poolboy).
        # Partitions from runtime.exs / test.exs; fallback only if key unset.
        {PartitionSupervisor,
         child_spec: {Assets.Blake3.Worker, []},
         name: Assets.Blake3.Partitioner,
         partitions: hash_partitions(:assets, :blake3_hash_partitions),
         with_arguments: fn [opts], partition ->
           [Keyword.put(opts, :partition, partition)]
         end}
      ] ++
        maybe_child(
          Application.get_env(:assets, :quota_reserved_reaper_start, true),
          Assets.Quota.ReservedReaper
        ) ++
        maybe_child(
          Application.get_env(:assets, :orphan_cas_gc_start, true),
          Assets.CAS.OrphanGc
        ) ++
        [
          # x0049: dedicated loopback Bandit — never on Endpoint (Tunnel peer spoof)
          Assets.Metrics.Server,
          AssetsWeb.Endpoint
        ]

    opts = [strategy: :one_for_one, name: Assets.Supervisor]
    Supervisor.start_link(children, opts)
  end

  defp maybe_child(true, child), do: [child]
  defp maybe_child(false, _child), do: []

  defp hash_partitions(app, key) do
    case Application.get_env(app, key) do
      n when is_integer(n) and n >= 1 -> n
      _ -> System.schedulers_online()
    end
  end

  @impl true
  def config_change(changed, _new, removed) do
    AssetsWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
