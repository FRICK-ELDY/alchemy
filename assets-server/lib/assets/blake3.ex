defmodule Assets.Blake3 do
  @moduledoc """
  BLAKE3 via permanent `blake3-hash --serve` Port (x0075; avoids Rustler NIF).

  Guarantees: hex digest of input bytes.
  Non-guarantees: streaming hash API; parallel throughput.

  Failures **raise** (upload path contract). World seal uses `{:error, _}` instead.
  """

  alias Assets.Blake3.Worker

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    Worker.hash_hex(data)
  end
end
