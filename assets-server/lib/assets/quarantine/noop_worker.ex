defmodule Assets.Quarantine.NoopWorker do
  @moduledoc """
  Quarantine worker that accepts every blob without inspection.

  Not the configured default (`HeaderCheckWorker` is). Use via
  `:assets, :quarantine_worker` when header checks are undesired;
  proprietary workers also inject through the same key.
  """

  @behaviour Assets.Quarantine.Worker

  @impl true
  def inspect_blob(_blake3, _data, _mime), do: :ok
end
