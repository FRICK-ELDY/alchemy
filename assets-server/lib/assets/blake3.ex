defmodule Assets.Blake3 do
  @moduledoc """
  BLAKE3 via the `b3sum` CLI (avoids Rustler NIF build fragility).

  Guarantees: hex digest of input bytes.
  Non-guarantees: streaming hash API.
  """

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    tmp =
      Path.join(
        System.tmp_dir!(),
        "alchemy-b3-#{System.unique_integer([:positive])}"
      )

    try do
      File.write!(tmp, data)

      case System.cmd("b3sum", ["--no-names", tmp], stderr_to_stdout: true) do
        {out, 0} ->
          out
          |> String.trim()
          |> String.downcase()
          |> validate_hex!()

        {out, code} ->
          raise "b3sum failed (exit #{code}): #{out}. Install with: cargo install b3sum"
      end
    after
      File.rm(tmp)
    end
  end

  defp validate_hex!(hex) do
    if Regex.match?(~r/^[0-9a-f]{64}$/, hex) do
      hex
    else
      raise "unexpected b3sum output: #{inspect(hex)}"
    end
  end
end
