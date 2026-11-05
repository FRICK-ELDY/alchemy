defmodule World.ManifestSealHash do
  @moduledoc """
  BLAKE3 of canonical SceneManifest protobuf via permanent `manifest-seal-hash --serve`
  Port (x0039 / x0075).

  Shares the Rust `alchemy-protocol` encode path with host/client without Rustler NIF.
  """

  @doc """
  Compute lowercase hex blake3 for a seal payload map.

  Payload keys: `room_id`, `revision`, `host_generation`, `protocol_pin`,
  `host_pub_hex`, `host_attestation`, `locomotion`, `entries`.
  """
  alias World.ManifestSealHash.Worker

  @spec blake3_hex(map()) :: {:ok, String.t()} | {:error, term()}
  def blake3_hex(payload) when is_map(payload) do
    json = Jason.encode!(payload)
    Worker.blake3_hex(json)
  end
end
