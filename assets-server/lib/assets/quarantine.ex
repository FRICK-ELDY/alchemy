defmodule Assets.Quarantine do
  @moduledoc """
  Entry point for quarantine inspection and stable public reject codes.

  Resolves `:assets, :quarantine_worker` (default: `HeaderCheckWorker`) and
  delegates. Proprietary pgvector workers must implement
  `Assets.Quarantine.Worker` and be configured outside this OSS tree.

  HTTP / API layers must use `public_reason/1` — never `inspect/1` on worker
  reasons (unknown / proprietary reasons collapse to `"rejected"`).
  """

  @type public_reason :: String.t()

  @spec inspect_blob(String.t(), binary(), String.t()) :: Assets.Quarantine.Worker.result()
  def inspect_blob(blake3, data, mime)
      when is_binary(blake3) and is_binary(data) and is_binary(mime) do
    worker().inspect_blob(blake3, data, mime)
  end

  @doc """
  Maps an internal quarantine reason to a stable public API code.

  Allowlist OSS reasons here when `HeaderCheckWorker` (or other OSS workers)
  adds new codes. Anything else — including proprietary worker payloads —
  becomes `"rejected"`.
  """
  @spec public_reason(term()) :: public_reason()
  def public_reason(:invalid_glb_header), do: "invalid_glb_header"
  def public_reason(_), do: "rejected"

  defp worker do
    Application.get_env(:assets, :quarantine_worker, Assets.Quarantine.HeaderCheckWorker)
  end
end
