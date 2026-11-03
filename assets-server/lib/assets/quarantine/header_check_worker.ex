defmodule Assets.Quarantine.HeaderCheckWorker do
  @moduledoc """
  Lightweight file-header validation for known asset MIME types.

  Guarantees: magic-byte checks only (e.g. GLB `glTF`); never computes
  perceptual hashes or pgvector similarity.
  Non-guarantees: mesh integrity, stolen-asset detection, deep format parse.
  """

  @behaviour Assets.Quarantine.Worker

  # glTF Binary (.glb) magic: ASCII "glTF"
  @glb_magic "glTF"

  @impl true
  def inspect_blob(_blake3, data, mime)
      when is_binary(data) and is_binary(mime) do
    if glb_mime?(mime) and not glb_header?(data) do
      {:quarantine, :invalid_glb_header}
    else
      :ok
    end
  end

  defp glb_mime?(mime) do
    # Only explicit glTF binary MIME — never treat generic octet-stream as GLB
    # (upload default mime is application/octet-stream).
    # Strip RFC 7231 parameters (`type/subtype; param=value`).
    media_type(mime) in ["model/gltf-binary", "model/gltf+binary"]
  end

  defp media_type(mime) do
    mime
    |> String.split(";", parts: 2)
    |> List.first()
    |> String.trim()
    |> String.downcase()
  end

  defp glb_header?(<<@glb_magic, _rest::binary>>), do: true
  defp glb_header?(_), do: false
end
