defmodule World.ManifestSealHash do
  @moduledoc """
  BLAKE3 of canonical SceneManifest protobuf via `manifest-seal-hash` CLI (x0039).

  Same pattern as assets `b3sum`: CLI avoids Rustler NIF fragility while sharing
  the Rust `alchemy-protocol` encode path with host/client.
  """

  @doc """
  Compute lowercase hex blake3 for a seal payload map.

  Payload keys: `room_id`, `revision`, `protocol_pin`, `host_pub_hex`,
  `host_attestation`, `locomotion`, `entries`.
  """
  @spec blake3_hex(map()) :: {:ok, String.t()} | {:error, term()}
  def blake3_hex(payload) when is_map(payload) do
    bin = binary_path()
    json = Jason.encode!(payload)

    tmp =
      Path.join(
        System.tmp_dir!(),
        "alchemy-manifest-seal-#{System.unique_integer([:positive])}.json"
      )

    try do
      File.write!(tmp, json)

      case System.cmd(bin, [tmp], stderr_to_stdout: true) do
        {out, 0} ->
          hex = out |> String.trim() |> String.downcase()

          if byte_size(hex) == 64 and Regex.match?(~r/^[0-9a-f]+$/, hex) do
            {:ok, hex}
          else
            {:error, {:unexpected_hash_output, out}}
          end

        {out, code} ->
          {:error, {:cli_failed, code, out}}
      end
    rescue
      e in ErlangError ->
        {:error, {:cli_missing, Exception.message(e)}}
    after
      _ = File.rm(tmp)
    end
  end

  defp binary_path do
    Application.get_env(:world, :manifest_seal_hash_bin) ||
      System.get_env("MANIFEST_SEAL_HASH_BIN") ||
      "manifest-seal-hash"
  end
end
