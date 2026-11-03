defmodule World.Verifier.AuditEngine do
  @moduledoc """
  Behaviour for host Spot-Check / anti-tamper audit heuristics.

  Guarantees: OSS (`world-server`) keeps only this injection surface plus
  ledger / liveliness / failover orchestration callers.
  Non-guarantees: concrete memory-tamper thresholds, statistical detectors,
  or proprietary scoring — those belong in private modules
  (`LicenseRef-Alchemy-Proprietary`) configured via
  `:world, :audit_engine`.
  """

  @type room_id :: String.t()
  @type host_sub :: String.t()
  @type frame_bytes :: binary()

  @typedoc "Opaque audit context supplied by the caller (generation, revision, etc.)."
  @type audit_context :: map()

  @typedoc "Outcome of a spot-check pass."
  @type result ::
          :ok
          | {:suspect, reason :: term()}
          | {:fail, reason :: term()}
          | {:error, term()}

  @callback spot_check(room_id(), host_sub(), frame_bytes(), audit_context()) :: result()
end
