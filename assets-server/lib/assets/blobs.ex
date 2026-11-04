defmodule Assets.Blobs do
  @moduledoc """
  Blob upload/download with quota reservation (x003f) and BlobGrant ledger (x004f).

  Flow: reserve (short TX) → CAS.put outside TX → finalize meta + owned grant + used.
  Failure after reserve releases reserved_bytes only. CAS bytes are never deleted
  on compensate — concurrent same-blake3 PUT (cross-owner PoP) must not race on
  delete. Orphan CAS objects without CasObject rows are out of scope (no GC yet).

  GET (x006c / x006f): always gated by BlobGrant for auth_sub **or** a covering
  PlaceRead JWT (x006b). CasObject holds bytes meta only (no `access` column).
  Inventory path (`POST /api/v1/inventory/blobs`) is an HTTP-boundary alias that
  uses the same upload (x0054 / x006f).
  """

  import Ecto.Query

  alias Assets.Blobs.BlobGrant
  alias Assets.Blobs.BlobRef
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.CAS.S3, as: CasS3
  alias Assets.Metrics
  alias Assets.PlaceRead
  alias Assets.Quota.QuotaLedger
  alias Assets.Repo

  @grant_owned "owned"

  @spec blake3_hex(binary()) :: String.t()
  def blake3_hex(data) when is_binary(data) do
    Assets.Blake3.hash_hex(data)
  end

  @doc """
  Upload bytes. Always issues an `:owned` BlobGrant for `owner_sub`.

  First CasObject writer wins for mime / size. Later PUT with the same blake3
  adds the caller's BlobRef + Grant without access conflict (x006f).
  """
  @spec upload(String.t(), binary(), String.t()) ::
          {:ok, CasObject.t(), :created | :existing} | {:error, term()}
  @spec upload(String.t(), binary(), String.t(), keyword()) ::
          {:ok, CasObject.t(), :created | :existing} | {:error, term()}
  def upload(owner_sub, data, mime, opts \\ [])
      when is_binary(owner_sub) and is_binary(data) and is_binary(mime) and is_list(opts) do
    # opts retained for call-site compatibility; access is ignored (x006f).
    _ = opts
    max = Application.fetch_env!(:assets, :max_upload_bytes)

    if byte_size(data) > max do
      Metrics.inc(:cas_put_errors_total)
      {:error, :too_large}
    else
      t0 = System.monotonic_time(:millisecond)

      result = do_upload(owner_sub, data, mime)
      Metrics.observe_ms(:cas_put_last_ms, System.monotonic_time(:millisecond) - t0)
      record_upload_metrics(result)
      result
    end
  end

  defp record_upload_metrics({:ok, _, _}), do: Metrics.inc(:cas_put_total)

  defp record_upload_metrics({:error, :quota_exceeded}) do
    Metrics.inc(:cas_quota_exceeded_total)
    Metrics.inc(:cas_put_errors_total)
  end

  defp record_upload_metrics({:error, {:quarantine, _}}), do: :ok
  defp record_upload_metrics({:error, _}), do: Metrics.inc(:cas_put_errors_total)

  defp do_upload(owner_sub, data, mime) do
    blake3 = blake3_hex(data)
    size = byte_size(data)
    mime = mime_or_default(mime)

    case Assets.Quarantine.inspect_blob(blake3, data, mime) do
      :ok ->
        continue_upload_after_quarantine(owner_sub, blake3, data, size, mime)

      {:quarantine, reason} ->
        {:error, {:quarantine, reason}}

      {:error, reason} ->
        {:error, {:quarantine_error, reason}}
    end
  end

  defp continue_upload_after_quarantine(owner_sub, blake3, data, size, mime) do
    case reserve_or_existing(owner_sub, blake3, size) do
      {:ok, :existing, object} ->
        # Ref already held — still ensure CAS bytes + owned grant (repair, no quota).
        ensure_cas_bytes(owner_sub, blake3, data, object)

      {:ok, :reserved} ->
        put_and_finalize(owner_sub, blake3, data, size, mime)

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

  defp put_and_finalize(owner_sub, blake3, data, size, mime) do
    # Always put_if_absent — even if CasObject row exists — so a missing CAS body
    # (orphan meta / prior crash) is repaired instead of skipping forever.
    case CAS.put_if_absent(blake3, data) do
      :created ->
        finalize_or_compensate(owner_sub, blake3, size, mime)

      :existing ->
        finalize_or_compensate(owner_sub, blake3, size, mime)

      {:error, reason} ->
        _ = release_reserved(owner_sub, size)
        {:error, {:cas_error, reason}}
    end
  end

  defp finalize_or_compensate(owner_sub, blake3, size, mime) do
    case maybe_fail_finalize_for_test(blake3) do
      :ok ->
        case finalize(owner_sub, blake3, size, mime) do
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

  defp finalize(owner_sub, blake3, size, mime) do
    Repo.transaction(fn ->
      object = upsert_cas_object!(blake3, size, mime)
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

  defp upsert_cas_object!(blake3, size, mime) do
    case Repo.get(CasObject, blake3) do
      # First writer wins for mime/size.
      %CasObject{} = object ->
        object

      nil ->
        insert_cas_object!(blake3, size, mime)
    end
  end

  defp insert_cas_object!(blake3, size, mime) do
    changeset =
      CasObject.changeset(%CasObject{}, %{
        blake3: blake3,
        size: size,
        mime: mime
      })

    case Repo.insert(changeset) do
      {:ok, object} -> object
      {:error, changeset} -> cas_object_insert_conflict!(blake3, changeset)
    end
  end

  defp cas_object_insert_conflict!(blake3, changeset) do
    if unique_constraint_conflict?(changeset) do
      fetch_cas_object!(blake3)
    else
      Repo.rollback({:invalid_cas_object, changeset})
    end
  end

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

  `auth_sub` is required (x004f / x006c / x006f). Read requires BlobGrant for
  `auth_sub` **or** covering PlaceRead via `opts[:place_read]` (x006b).
  """
  @spec fetch(String.t(), String.t()) ::
          {:ok, binary(), CasObject.t()}
          | {:redirect, String.t(), CasObject.t()}
          | {:error, :not_found | :forbidden | :presign_failed}
  @spec fetch(String.t(), String.t(), keyword()) ::
          {:ok, binary(), CasObject.t()}
          | {:redirect, String.t(), CasObject.t()}
          | {:error, :not_found | :forbidden | :presign_failed}
  def fetch(blake3, auth_sub, opts \\ [])

  def fetch(blake3, auth_sub, opts)
      when is_binary(blake3) and is_binary(auth_sub) and is_list(opts) do
    t0 = System.monotonic_time(:millisecond)
    place_read = Keyword.get(opts, :place_read)

    result =
      case Repo.get(CasObject, blake3) do
        nil ->
          {:error, :not_found}

        %CasObject{} = object ->
          authorize_and_fetch(object, blake3, auth_sub, place_read)
      end

    Metrics.observe_ms(:cas_get_last_ms, System.monotonic_time(:millisecond) - t0)
    record_fetch_metrics(result)
    result
  end

  defp authorize_and_fetch(%CasObject{} = object, blake3, auth_sub, place_read) do
    cond do
      has_blob_grant?(auth_sub, blake3) ->
        fetch_object(object, blake3)

      is_binary(place_read) and PlaceRead.allows_blake3?(place_read, auth_sub, blake3) ->
        fetch_object(object, blake3)

      true ->
        {:error, :forbidden}
    end
  end

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
