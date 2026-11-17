defmodule WorldWeb.FallbackController do
  @moduledoc """
  Shared `action_fallback` error table (x00d6 / P2-8).

  **Current scope:** Room API vocabulary dominates this table because
  `RoomController` is the only consumer that returns `{:error, _}`.
  `RelationController` (and others) still render inline. If Relation adopts
  fallback later, either keep generic atoms here (`:not_found` → `"not_found"`)
  and Room-specific atoms (`:room_not_found`), or split a `RoomFallbackController`.

  Controllers must normalize ambiguous domain atoms (`:conflict`, `:forbidden_host`,
  domain `:not_found` → `:room_not_found`, …) before falling through. SpawnTicket
  release and similar side effects run **before** this module.
  """
  use WorldWeb, :controller

  require Logger

  # Unique atom → {Plug status atom | integer, JSON error string}
  @table %{
    # 401
    anon_disabled: {:unauthorized, "anon_tokens_disabled"},
    missing_bearer: {:unauthorized, "missing_room_token"},
    missing_place_read: {:unauthorized, "missing_place_read"},
    pop_required: {:unauthorized, "pop_required"},
    invalid_room_token: {:unauthorized, "invalid_room_token"},
    invalid_place_read: {:unauthorized, "invalid_place_read"},
    invalid_place_read_request: {:unauthorized, "invalid_place_read_request"},
    # 403
    room_mismatch: {:forbidden, "room_id_mismatch"},
    # HTTP-boundary atom (domain `:forbidden_visibility` is mapped in RoomController)
    visibility_forbidden: {:forbidden, "visibility_forbidden"},
    owner_only: {:forbidden, "owner_only"},
    host_role_forbidden: {:forbidden, "host_role_forbidden"},
    host_role_required: {:forbidden, "host_role_required"},
    not_host_token: {:forbidden, "host_role_required"},
    handover_forbidden: {:forbidden, "handover_forbidden"},
    handover_accept_forbidden: {:forbidden, "handover_accept_forbidden"},
    takeover_forbidden: {:forbidden, "takeover_forbidden"},
    lan_required: {:forbidden, "lan_required"},
    spawn_ticket_required: {:forbidden, "spawn_ticket_required"},
    spawn_ticket_invalid: {:forbidden, "spawn_ticket_invalid"},
    forbidden_scene_writer: {:forbidden, "scene_writer_forbidden"},
    attestation_room_mismatch: {:forbidden, "attestation_room_mismatch"},
    seal_room_mismatch: {:forbidden, "seal_room_mismatch"},
    owner_room_cap_exceeded: {:forbidden, "room_cap_exceeded"},
    # 404 — generic vs Room-specific (do not collapse shared `:not_found` into Room JSON)
    not_found: {:not_found, "not_found"},
    room_not_found: {:not_found, "room_not_found"},
    entry_not_found: {:not_found, "entry_not_found"},
    # 409
    already_exists: {:conflict, "room_already_exists"},
    host_claim_conflict: {:conflict, "host_claim_conflict"},
    handover_conflict: {:conflict, "handover_conflict"},
    handover_accept_conflict: {:conflict, "handover_accept_conflict"},
    takeover_conflict: {:conflict, "takeover_conflict"},
    host_lease_conflict: {:conflict, "host_lease_conflict"},
    scene_revision_conflict: {:conflict, "scene_revision_conflict"},
    duplicate_blake3: {:conflict, "duplicate_blake3"},
    place_hashes_unavailable: {:conflict, "place_hashes_unavailable"},
    # 413
    scene_entries_exceeded: {:request_entity_too_large, "scene_budget_exceeded"},
    scene_bytes_exceeded: {:request_entity_too_large, "scene_budget_exceeded"},
    csr_pem_too_large: {:request_entity_too_large, "csr_pem_too_large"},
    # 422
    invalid_room_id: {:unprocessable_entity, "invalid_room_id"},
    invalid_visibility: {:unprocessable_entity, "invalid_visibility"},
    missing_owner: {:unprocessable_entity, "missing_owner"},
    invalid_eligible: {:unprocessable_entity, "invalid_eligible"},
    invalid_session_pub: {:unprocessable_entity, "invalid_session_pub"},
    invalid_role: {:unprocessable_entity, "invalid_role"},
    invalid_allocation_args: {:unprocessable_entity, "invalid_allocation_args"},
    invalid_args: {:unprocessable_entity, "invalid_csr_pem"},
    invalid_revision: {:unprocessable_entity, "invalid_revision"},
    invalid_target: {:unprocessable_entity, "invalid_target"},
    invalid_host_pub: {:unprocessable_entity, "invalid_host_pub"},
    invalid_entity_id: {:unprocessable_entity, "invalid_entity_id"},
    not_mesh_entry: {:unprocessable_entity, "not_mesh_entry"},
    ambiguous_entity_id: {:unprocessable_entity, "ambiguous_entity_id"},
    invalid_entry: {:unprocessable_entity, "invalid_entry"},
    parent_cycle: {:unprocessable_entity, "parent_cycle"},
    revision_mismatch: {:unprocessable_entity, "revision_mismatch"},
    invalid_manifest_blake3: {:unprocessable_entity, "invalid_manifest_blake3"},
    manifest_blake3_mismatch: {:unprocessable_entity, "manifest_blake3_mismatch"},
    missing_host_attestation: {:unprocessable_entity, "missing_host_attestation"},
    invalid_host_attestation: {:unprocessable_entity, "invalid_host_attestation"},
    stale_host_attestation: {:unprocessable_entity, "stale_host_attestation"},
    missing_world_seal: {:unprocessable_entity, "missing_world_seal"},
    extra_blake3_disabled: {:unprocessable_entity, "extra_blake3_disabled"},
    invalid_extra_blake3: {:unprocessable_entity, "invalid_extra_blake3"},
    seal_expired: {:unprocessable_entity, "seal_expired"},
    invalid_seal_exp: {:unprocessable_entity, "invalid_world_seal"},
    # 500 (client-facing labels kept)
    invalid_revision_server: {:internal_server_error, "invalid_revision"},
    openssl_failed: {:internal_server_error, "mtls_sign_failed"},
    sign_failed: {:internal_server_error, "mtls_sign_failed"},
    chmod_failed: {:internal_server_error, "mtls_sign_failed"},
    invalid_room_key: {:internal_server_error, "invalid_room_key"},
    token_issue_failed: {:internal_server_error, "token_issue_failed"},
    attestation_issue_failed: {:internal_server_error, "attestation_issue_failed"},
    invalid_scene_revision: {:internal_server_error, "invalid_scene_revision"},
    manifest_seal_issue_failed: {:internal_server_error, "manifest_seal_issue_failed"},
    manifest_seal_hash_unavailable: {:internal_server_error, "manifest_seal_hash_unavailable"},
    manifest_seal_hash_failed: {:internal_server_error, "manifest_seal_hash_failed"},
    # 503
    storage_error: {:service_unavailable, "storage_unavailable"},
    auth_unavailable: {:service_unavailable, "auth_access_unavailable"}
  }

  def call(conn, {:error, reason}) when is_map_key(@table, reason) do
    {status, error} = Map.fetch!(@table, reason)

    conn
    |> put_status(status)
    |> json(%{error: error})
  end

  def call(conn, {:error, {:invalid_room_token, log_ctx, reason}}) do
    Logger.warning("invalid Room Token (#{log_ctx} path): #{inspect(reason)}")

    conn
    |> put_status(:unauthorized)
    |> json(%{error: "invalid_room_token"})
  end

  def call(conn, {:error, {:token_issue_failed, reason}}) do
    Logger.error("Room Token issue failed: #{inspect(reason)}")

    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "token_issue_failed"})
  end

  def call(conn, {:error, {:attestation_issue_failed, reason}}) do
    Logger.error("HostAttestation.generate failed: #{inspect(reason)}")

    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "attestation_issue_failed"})
  end

  def call(conn, {:error, {:manifest_seal_issue_failed, reason}}) do
    Logger.error("ManifestSeal.generate failed: #{inspect(reason)}")

    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "manifest_seal_issue_failed"})
  end

  def call(conn, {:error, {:cli_missing, _}}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "manifest_seal_hash_unavailable"})
  end

  def call(conn, {:error, {:cli_failed, _, _}}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "manifest_seal_hash_failed"})
  end

  def call(conn, {:error, {:unexpected_hash_output, _}}) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "manifest_seal_hash_failed"})
  end

  def call(conn, {:error, {:invalid_world_seal, _}}) do
    conn
    |> put_status(:unprocessable_entity)
    |> json(%{error: "invalid_world_seal"})
  end

  def call(conn, _error) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "internal_error"})
  end
end
