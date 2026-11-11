defmodule Assets.Blake3 do
  @moduledoc """
  BLAKE3 via permanent `blake3-hash --serve` Port pool (x0075 / x008d; avoids Rustler NIF).

  Guarantees: hex digest of input bytes; up to `:blake3_hash_partitions` concurrent
  hashes (boot-time `System.schedulers_online/0` via runtime.exs / env).
  Non-guarantees: streaming hash API; more-than-N parallelism.

  Failures **raise** (upload path contract), including pool `:noproc` / call timeout.
  World seal uses `{:error, _}` instead.
  """

  alias Assets.Blake3.Worker

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    Worker.hash_hex(data)
  end
end
