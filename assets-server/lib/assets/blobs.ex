defmodule Assets.Blobs do
  @moduledoc """
  Blob upload/download with quota reservation (x003f) and BlobGrant ledger (x004f).

  Flow: reserve (short TX) → CAS.put outside TX → finalize meta + owned grant + used.
  Failure after reserve releases reserved_bytes only. CAS bytes are never deleted
  on compensate — concurrent same-blake3 PUT (cross-owner PoP) must not race on
  delete. Orphan CAS objects without CasObject rows are out of scope (no GC yet).

  GET: default `access=authenticated` (Bearer + CasObject). `grant_required` needs
  a BlobGrant for auth_sub. Target end-state (option B): all GET gated by grant.
  """

  import Ecto.Query

  alias Assets.Blobs.BlobGrant
  alias Assets.Blobs.BlobRef
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.CAS.S3, as: CasS3
  alias Assets.Metrics
  alias Assets.Quota.QuotaLedger
  alias Assets.Repo

  @access_authenticated "authenticated"
  @access_grant_required "grant_required"
  @grant_owned "owned"

  @spec blake3_hex(binary()) :: String.t()
  def blake3_hex(data) when is_binary(data) do
    Assets.Blake3.hash_hex(data)
  end

  @doc """
  Upload bytes. `opts[:access]` is `:authenticated` (default) or `:grant_required`.

  First CasObject writer wins for `access` / mime / size. A later PUT that requests a
  different `access` than the existing object returns `{:error, :access_conflict}`
  (HTTP 409); reserved quota is released and no BlobRef/Grant is created for the loser.
  """
  @spec upload(String.t(), binary(), String.t(), keyword()) ::
          {:ok, CasObject.t(), :created | :existing} | {:error, term()}
  def upload(owner_sub, data, mime, opts \\ [])
      when is_binary(owner_sub) and is_binary(data) and is_binary(mime) and is_list(opts) do
    max = Application.fetch_env!(:assets, :max_upload_bytes)

    with {:ok, access} <- normalize_access(Keyword.get(opts, :access, :authenticated)) do
      if byte_size(data) > max do
        Metrics.inc(:cas_put_errors_total)
        {:error, :too_large}
      else
        t0 = System.monotonic_time(:millisecond)

        result = do_upload(owner_sub, data, mime, access)
        Metrics.observe_ms(:cas_put_last_ms, System.monotonic_time(:millisecond) - t0)
        record_upload_metrics(result)
        result
      end
    end
  end

  defp record_upload_metrics({:ok, _, _}), do: Metrics.inc(:cas_put_total)

  defp record_upload_metrics({:error, :quota_exceeded}) do
    Metrics.inc(:cas_quota_exceeded_total)
    Metrics.inc(:cas_put_errors_total)
  end

  # Client access race / first-writer conflict is not an infra put failure.
  defp record_upload_metrics({:error, :access_conflict}), do: :ok
  defp record_upload_metrics({:error, _}), do: Metrics.inc(:cas_put_errors_total)

  defp do_upload(owner_sub, data, mime, access) do
    blake3 = blake3_hex(data)
    size = byte_size(data)
    mime = mime_or_default(mime)

    case reserve_or_existing(owner_sub, blake3, size) do
      {:ok, :existing, object} ->
        # Ref already held — still ensure CAS bytes + owned grant (repair, no quota).
        # access is first-writer; mismatch must not look like success.
        with :ok <- assert_access_match(object, access) do
          ensure_cas_bytes(owner_sub, blake3, data, object)
        end

      {:ok, :reserved} ->
        put_and_finalize(owner_sub, blake3, data, size, mime, access)

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp ensure_cas_bytes(owner_sub, blake3, data, object) do
    _ = insert_blob_grant(owner_sub, blake3)

    case CAS.put_if_absent(blake3, data) do
      :created -> {:ok, object, :existing}
      :existing -> {:ok, object, :existing}
      {:error, reason} -> {:error, {:cas_error, reason}}
    end
  end

  defp reserve_or_existing(owner_sub, blake3, size) do
    case Repo.transaction(fn -> reserve_tx!(owner_sub, blake3, size) end) do
      {:ok, {:existing, object}} -> {:ok, :existing, object}
      {:ok, :reserved} -> {:ok, :reserved}
      {:error, :quota_exceeded} -> recover_existing_after_quota(owner_sub, blake3)
      {:error, reason} -> {:error, reason}
    end
  end

  defp reserve_tx!(owner_sub, blake3, size) do
    case get_blob_ref(owner_sub, blake3) do
      %BlobRef{} -> existing_object_or_rollback!(blake3)
      nil -> reserve_new_ref!(owner_sub, size)
    end
  end

  defp existing_object_or_rollback!(blake3) do
    case Repo.get(CasObject, blake3) do
      %CasObject{} = object -> {:existing, object}
      nil -> Repo.rollback({:missing_cas_object, blake3})
    end
  end

  defp reserve_new_ref!(owner_sub, size) do
    ensure_quota_ledger!(owner_sub)
    reserve_quota!(owner_sub, size)
    :reserved
  end

  # Same-owner concurrent PUT: peer may already own the ref. Mid-flight (ref not
  # visible yet) still returns quota_exceeded — client retry yields :existing.
  defp recover_existing_after_quota(owner_sub, blake3) do
    case existing_ref_object(owner_sub, blake3) do
      %CasObject{} = object -> {:ok, :existing, object}
      nil -> {:error, :quota_exceeded}
    end
  end

  defp existing_ref_object(owner_sub, blake3) do
    with %BlobRef{} <- get_blob_ref(owner_sub, blake3),
         %CasObject{} = object <- Repo.get(CasObject, blake3) do
      object
    else
      _ -> nil
    end
  end

  defp put_and_finalize(owner_sub, blake3, data, size, mime, access) do
    # Always put_if_absent — even if CasObject row exists — so a missing CAS body
    # (orphan meta / prior crash) is repaired instead of skipping forever.
    case CAS.put_if_absent(blake3, data) do
      :created ->
        finalize_or_compensate(owner_sub, blake3, size, mime, access)

      :existing ->
        finalize_or_compensate(owner_sub, blake3, size, mime, access)

      {:error, reason} ->
        _ = release_reserved(owner_sub, size)
        {:error, {:cas_error, reason}}
    end
  end

  defp finalize_or_compensate(owner_sub, blake3, size, mime, access) do
    case maybe_fail_finalize_for_test(blake3) do
      :ok ->
        case finalize(owner_sub, blake3, size, mime, access) do
          {:ok, object, status} ->
            {:ok, object, status}

          {:error, reason} ->
            compensate(owner_sub, size)
            {:error, reason}
        end

      {:error, reason} ->
        compensate(owner_sub, size)
        {:error, reason}
    end
  end

  # Test-only hook. Mix is absent in releases; never trip compensate via env alone.
  defp maybe_fail_finalize_for_test(blake3) do
    if function_exported?(Mix, :env, 0) and Mix.env() == :test and
         Application.get_env(:assets, :fail_finalize_blake3) == blake3 do
      {:error, :forced_finalize_failure}
    else
      :ok
    end
  end

  defp finalize(owner_sub, blake3, size, mime, access) do
    Repo.transaction(fn ->
      object = upsert_cas_object!(blake3, size, mime, access)
      _ = insert_blob_grant(owner_sub, blake3)

      case insert_blob_ref(owner_sub, blake3) do
        {:ok, :created} ->
          commit_reserved!(owner_sub, size)
          {:created, object}

        {:ok, :existing} ->
          release_quota_in_tx!(owner_sub, size)
          {:existing, object}
      end
    end)
    |> case do
      {:ok, {:created, object}} -> {:ok, object, :created}
      {:ok, {:existing, object}} -> {:ok, object, :existing}
      {:error, reason} -> {:error, reason}
    end
  end

  # Quota only. Never CAS.delete — concurrent putters of the same blake3 share bytes.
  defp compensate(owner_sub, size) do
    release_reserved(owner_sub, size)
    :ok
  end

  defp upsert_cas_object!(blake3, size, mime, access) do
    case Repo.get(CasObject, blake3) do
      # First writer wins for mime/size/access. Later PUT must request the same access
      # or fail with :access_conflict (do not silently ignore grant_required).
      %CasObject{} = object ->
        case assert_access_match(object, access) do
          :ok -> object
          {:error, :access_conflict} -> Repo.rollback(:access_conflict)
        end

      nil ->
        insert_cas_object!(blake3, size, mime, access)
    end
  end

  defp insert_cas_object!(blake3, size, mime, access) do
    changeset =
      CasObject.changeset(%CasObject{}, %{
        blake3: blake3,
        size: size,
        mime: mime,
        access: access
      })

    case Repo.insert(changeset) do
      {:ok, object} -> object
      {:error, changeset} -> cas_object_insert_conflict!(blake3, access, changeset)
    end
  end

  defp cas_object_insert_conflict!(blake3, access, changeset) do
    if unique_constraint_conflict?(changeset) do
      object = fetch_cas_object!(blake3)

      case assert_access_match(object, access) do
        :ok -> object
        {:error, :access_conflict} -> Repo.rollback(:access_conflict)
      end
    else
      Repo.rollback({:invalid_cas_object, changeset})
    end
  end

  defp assert_access_match(%CasObject{access: existing}, requested)
       when existing == requested,
       do: :ok

  defp assert_access_match(%CasObject{}, _requested), do: {:error, :access_conflict}

  defp fetch_cas_object!(blake3) do
    case Repo.get(CasObject, blake3) do
      %CasObject{} = object -> object
      nil -> Repo.rollback({:missing_cas_object, blake3})
    end
  end

  defp insert_blob_ref(owner_sub, blake3) do
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    # insert_all + on_conflict avoids depending on which field SQLite maps the PK error to.
    # conflict_target is required for PostgreSQL (Stage 2); SQLite accepts it too.
    {n, _} =
      Repo.insert_all(
        BlobRef,
        [%{owner_sub: owner_sub, blake3: blake3, inserted_at: now}],
        on_conflict: :nothing,
        conflict_target: [:owner_sub, :blake3]
      )

    if n == 1 do
      {:ok, :created}
    else
      {:ok, :existing}
    end
  end

  defp insert_blob_grant(grantee_sub, blake3) do
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    {n, _} =
      Repo.insert_all(
        BlobGrant,
        [
          %{
            grantee_sub: grantee_sub,
            blake3: blake3,
            grant_type: @grant_owned,
            inserted_at: now
          }
        ],
        on_conflict: :nothing,
        conflict_target: [:grantee_sub, :blake3]
      )

    if n == 1, do: :created, else: :existing
  end

  defp has_blob_grant?(grantee_sub, blake3) do
    match?(
      %BlobGrant{},
      Repo.get_by(BlobGrant, grantee_sub: grantee_sub, blake3: blake3)
    )
  end

  defp normalize_access(:authenticated), do: {:ok, @access_authenticated}
  defp normalize_access("authenticated"), do: {:ok, @access_authenticated}
  defp normalize_access(:grant_required), do: {:ok, @access_grant_required}
  defp normalize_access("grant_required"), do: {:ok, @access_grant_required}
  defp normalize_access(_), do: {:error, :invalid_access}

  defp unique_constraint_conflict?(%Ecto.Changeset{errors: errors}) do
    Enum.any?(errors, fn
      {_field, {_, opts}} ->
        opts[:constraint] == :unique or
          opts[:constraint_name] in [:cas_objects_pkey, "cas_objects_pkey"]

      _ ->
        false
    end)
  end

  defp get_blob_ref(owner_sub, blake3) do
    Repo.get_by(BlobRef, owner_sub: owner_sub, blake3: blake3)
  end

  @doc """
  Resolve blob for HTTP GET: local proxies bytes; s3 returns a signed redirect URL (x001d).

  `auth_sub` is required for access checks (x004f). Fail-closed: only known `access`
  values are served (`authenticated` capability, or `grant_required` with BlobGrant).
  Unknown/corrupt `access` → `:forbidden` (never treat as open).
  """
  @spec fetch(String.t(), String.t()) ::
          {:ok, binary(), CasObject.t()}
          | {:redirect, String.t(), CasObject.t()}
          | {:error, :not_found | :forbidden | :presign_failed}
  def fetch(blake3, auth_sub)
      when is_binary(blake3) and is_binary(auth_sub) do
    t0 = System.monotonic_time(:millisecond)

    result =
      case Repo.get(CasObject, blake3) do
        nil ->
          {:error, :not_found}

        %CasObject{} = object ->
          authorize_and_fetch(object, blake3, auth_sub)
      end

    Metrics.observe_ms(:cas_get_last_ms, System.monotonic_time(:millisecond) - t0)
    record_fetch_metrics(result)
    result
  end

  defp authorize_and_fetch(
         %CasObject{access: @access_authenticated} = object,
         blake3,
         _auth_sub
       ) do
    fetch_object(object, blake3)
  end

  defp authorize_and_fetch(
         %CasObject{access: @access_grant_required} = object,
         blake3,
         auth_sub
       ) do
    if has_blob_grant?(auth_sub, blake3) do
      fetch_object(object, blake3)
    else
      {:error, :forbidden}
    end
  end

  # Unknown access must not fall open to authenticated capability.
  defp authorize_and_fetch(%CasObject{}, _blake3, _auth_sub), do: {:error, :forbidden}

  defp record_fetch_metrics({:ok, _, _}), do: Metrics.inc(:cas_get_total)
  defp record_fetch_metrics({:redirect, _, _}), do: Metrics.inc(:cas_get_total)
  # Expected authz denial is not an infra/get error.
  defp record_fetch_metrics({:error, :forbidden}), do: :ok
  defp record_fetch_metrics({:error, _}), do: Metrics.inc(:cas_get_errors_total)

  defp fetch_object(object, blake3) do
    case Application.fetch_env!(:assets, :cas_backend) do
      :local -> fetch_local(object, blake3)
      :s3 -> fetch_s3_redirect(object, blake3)
    end
  end

  defp fetch_local(object, blake3) do
    case CAS.read(blake3) do
      {:ok, data} -> {:ok, data, object}
      {:error, _} -> {:error, :not_found}
    end
  end

  defp fetch_s3_redirect(object, blake3) do
    case CasS3.presign_get(blake3) do
      {:ok, url} -> {:redirect, url, object}
      {:error, _} -> {:error, :presign_failed}
    end
  end

  @spec quota(String.t()) :: %{
          used_bytes: non_neg_integer(),
          reserved_bytes: non_neg_integer(),
          limit_bytes: pos_integer()
        }
  def quota(owner_sub) when is_binary(owner_sub) do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    case Repo.get(QuotaLedger, owner_sub) do
      %QuotaLedger{used_bytes: used, reserved_bytes: reserved, limit_bytes: lim} ->
        %{used_bytes: used, reserved_bytes: reserved, limit_bytes: lim}

      nil ->
        %{used_bytes: 0, reserved_bytes: 0, limit_bytes: limit}
    end
  end

  defp ensure_quota_ledger!(owner_sub) do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    changeset =
      QuotaLedger.changeset(%QuotaLedger{}, %{
        owner_sub: owner_sub,
        used_bytes: 0,
        reserved_bytes: 0,
        limit_bytes: limit
      })

    case Repo.insert(changeset, on_conflict: :nothing, conflict_target: :owner_sub) do
      {:ok, _} ->
        :ok

      {:error, reason} ->
        Repo.rollback({:quota_ledger_error, reason})
    end
  end

  # Atomic: only increments when used + reserved + add <= limit.
  defp reserve_quota!(owner_sub, add_bytes) when add_bytes >= 0 do
    {n, _} =
      from(q in QuotaLedger,
        where:
          q.owner_sub == ^owner_sub and
            q.used_bytes + q.reserved_bytes + ^add_bytes <= q.limit_bytes
      )
      |> Repo.update_all(inc: [reserved_bytes: add_bytes])

    if n == 1 do
      :ok
    else
      Repo.rollback(:quota_exceeded)
    end
  end

  defp commit_reserved!(owner_sub, size) when size >= 0 do
    {n, _} =
      from(q in QuotaLedger,
        where: q.owner_sub == ^owner_sub and q.reserved_bytes >= ^size
      )
      |> Repo.update_all(inc: [reserved_bytes: -size, used_bytes: size])

    if n == 1 do
      :ok
    else
      Repo.rollback({:quota_commit_failed, owner_sub})
    end
  end

  defp release_quota_in_tx!(owner_sub, size) when size >= 0 do
    {n, _} =
      from(q in QuotaLedger,
        where: q.owner_sub == ^owner_sub and q.reserved_bytes >= ^size
      )
      |> Repo.update_all(inc: [reserved_bytes: -size])

    if n == 1 do
      :ok
    else
      Repo.rollback({:quota_release_failed, owner_sub})
    end
  end

  defp release_reserved(owner_sub, size) when size >= 0 do
    from(q in QuotaLedger,
      where: q.owner_sub == ^owner_sub and q.reserved_bytes >= ^size
    )
    |> Repo.update_all(inc: [reserved_bytes: -size])

    :ok
  end

  defp mime_or_default(""), do: "application/octet-stream"
  defp mime_or_default(nil), do: "application/octet-stream"
  defp mime_or_default(mime), do: mime
end
