defmodule Assets.Vcs.Cert do
  @moduledoc """
  VcsCert — Origin Certificate v2 (x00ca / draft §11.4).

  Canonical messages:

      `v2|commit|{commit_hash}|{author_sub}|{issued_at_ms}|{kid}`
      `v2|release|{repo_id}|{tag_name}|{commit_hash}|{signer_sub}|{issued_at_ms}|{kid}`

  ## Flow (TX-outside prepare → TX-inside materialize)

  Binding checks are intentional fail-closed at both stages. Public `issue_*`
  also checks Commit/tag/owner before prepare.

  ```mermaid
  sequenceDiagram
    participant Caller
    participant Cert
    participant DB
    Caller->>Cert: prepare_unless_present
    alt rowExists
      Cert->>DB: get existing row
      Cert-->>Caller: existing or cert_conflict
    else firstIssue
      Cert-->>Caller: signed attrs no DB write
    end
    Caller->>DB: begin TX writes
    Caller->>Cert: insert_prepared
    Cert->>DB: recheck author or owner plus tag
    alt alreadyExisting
      Cert-->>Caller: same row
    else newAttrs
      Cert->>DB: insert or get unique
      Cert-->>Caller: cert row
    end
  ```

  Guarantees:
  - Authorship / Release issue+verify fail-closed; private key never logged.
  - Authorship subject must equal `Commit.author_sub`; at most one row per commit_hash
    (re-issue returns the existing row; subject/`target_id` mismatch → `:cert_conflict`).
  - Release subject must equal `VcsRepo.owner_sub`; tag must exist and match commit_hash;
    at most one row per `(repo_id, tag_name)`.
  - Offline verify: message + signature + **caller-supplied** Ed25519 public key
    (Base64). When `pub_key_b64` is omitted, the current `:cert_signing_pub_key` is used.

  Non-guarantees / kid contract:
  - v1 compatibility; CA chain; HTTP export; multi-key JWKS / automated rotation.
  - `kid` is bound into the signed message for bookkeeping. Verify paths do **not**
    look up historical keys by `kid` — after rotation, pass the matching public key
    to `verify_offline_*`, or keep the issuing key configured as current for `verify/1`.
  """

  use Ecto.Schema
  import Ecto.Changeset
  import Ecto.Query, only: [from: 2]

  alias Assets.Repo
  alias Assets.Vcs
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo

  @cert_kind_values ~w(commit release)
  @message_version "v2"
  @default_kid "default"

  @primary_key {:cert_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_certs" do
    field :subject_sub, :string
    field :cert_kind, :string
    field :target_id, :string
    field :repo_id, :string
    field :tag_name, :string
    field :issued_at_ms, :integer
    field :kid, :string
    field :message_version, :string
    field :target_hash, :string
    field :signature, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(cert, attrs) do
    cert
    |> cast(attrs, [
      :cert_id,
      :subject_sub,
      :cert_kind,
      :target_id,
      :repo_id,
      :tag_name,
      :issued_at_ms,
      :kid,
      :message_version,
      :target_hash,
      :signature
    ])
    |> validate_required([
      :cert_id,
      :subject_sub,
      :cert_kind,
      :target_id,
      :issued_at_ms,
      :kid,
      :message_version,
      :target_hash,
      :signature
    ])
    |> validate_inclusion(:cert_kind, @cert_kind_values)
    |> validate_release_fields()
    |> unique_constraint(:target_id, name: :vcs_certs_authorship_commit_index)
    |> unique_constraint([:repo_id, :tag_name], name: :vcs_certs_release_repo_tag_index)
  end

  defp validate_release_fields(changeset) do
    case get_field(changeset, :cert_kind) do
      "release" ->
        changeset
        |> validate_required([:repo_id, :tag_name])

      _ ->
        changeset
    end
  end

  @type t :: %__MODULE__{}
  @type signed_attrs :: %{
          cert_id: String.t(),
          subject_sub: String.t(),
          cert_kind: String.t(),
          target_id: String.t(),
          repo_id: String.t() | nil,
          tag_name: String.t() | nil,
          issued_at_ms: non_neg_integer(),
          kid: String.t(),
          message_version: String.t(),
          target_hash: String.t(),
          signature: String.t()
        }

  @doc """
  Issue Authorship for an existing commit. `author_sub` must equal `Commit.author_sub`.
  If an Authorship row already exists for the commit_hash, returns that row (idempotent).
  """
  @spec issue_authorship(String.t(), String.t()) :: {:ok, t()} | {:error, term()}
  def issue_authorship(author_sub, commit_hash)
      when is_binary(author_sub) and is_binary(commit_hash) do
    with :ok <- validate_pipe_free(author_sub, :invalid_subject_sub),
         {:ok, commit} <- Vcs.get_commit(commit_hash),
         :ok <- assert_author_match(commit, author_sub),
         {:ok, prepared} <- prepare_authorship_unless_present(author_sub, commit_hash) do
      materialize_authorship(prepared)
    end
  end

  def issue_authorship(author_sub, _commit_hash) when not is_binary(author_sub) do
    {:error, :invalid_subject_sub}
  end

  def issue_authorship(_author_sub, _commit_hash), do: {:error, :invalid_commit_hash}

  @doc """
  Issue Release for an existing tag. `signer_sub` must equal repo owner; tag must
  exist and point at `commit_hash`. Idempotent on `(repo_id, tag_name)`.
  """
  @spec issue_release(String.t(), String.t(), String.t(), String.t()) ::
          {:ok, t()} | {:error, term()}
  def issue_release(signer_sub, repo_id, tag_name, commit_hash)
      when is_binary(signer_sub) and is_binary(repo_id) and is_binary(tag_name) and
             is_binary(commit_hash) do
    with :ok <- validate_pipe_free(signer_sub, :invalid_subject_sub),
         :ok <- validate_pipe_free(tag_name, :invalid_tag_name),
         :ok <- validate_pipe_free(repo_id, :invalid_repo_id),
         {:ok, _commit} <- Vcs.get_commit(commit_hash),
         :ok <- assert_release_binding(signer_sub, repo_id, tag_name, commit_hash),
         {:ok, prepared} <-
           prepare_release_unless_present(signer_sub, repo_id, tag_name, commit_hash) do
      materialize_release(prepared)
    end
  end

  def issue_release(signer_sub, _repo_id, _tag_name, _commit_hash)
      when not is_binary(signer_sub) do
    {:error, :invalid_subject_sub}
  end

  def issue_release(_signer_sub, _repo_id, _tag_name, _commit_hash) do
    {:error, :invalid_release_args}
  end

  @doc false
  @spec prepare_authorship_unless_present(String.t(), String.t()) ::
          {:ok, {:existing, t()} | {:attrs, signed_attrs()}} | {:error, term()}
  def prepare_authorship_unless_present(author_sub, commit_hash)
      when is_binary(author_sub) and is_binary(commit_hash) do
    case get_authorship(commit_hash) do
      %__MODULE__{} = existing ->
        with :ok <- assert_existing_authorship(existing, author_sub, commit_hash) do
          {:ok, {:existing, existing}}
        end

      nil ->
        with {:ok, attrs} <- sign_authorship(author_sub, commit_hash) do
          {:ok, {:attrs, attrs}}
        end
    end
  end

  @doc false
  @spec prepare_release_unless_present(String.t(), String.t(), String.t(), String.t()) ::
          {:ok, {:existing, t()} | {:attrs, signed_attrs()}} | {:error, term()}
  def prepare_release_unless_present(signer_sub, repo_id, tag_name, commit_hash)
      when is_binary(signer_sub) and is_binary(repo_id) and is_binary(tag_name) and
             is_binary(commit_hash) do
    case get_release(repo_id, tag_name) do
      %__MODULE__{} = existing ->
        with :ok <-
               assert_existing_release(existing, signer_sub, repo_id, tag_name, commit_hash) do
          {:ok, {:existing, existing}}
        end

      nil ->
        with {:ok, attrs} <- sign_release(signer_sub, repo_id, tag_name, commit_hash) do
          {:ok, {:attrs, attrs}}
        end
    end
  end

  @doc false
  @spec insert_prepared_authorship!({:existing, t()} | {:attrs, signed_attrs()}) :: t()
  def insert_prepared_authorship!({:existing, %__MODULE__{} = cert}) do
    case recheck_authorship_row(cert) do
      :ok -> cert
      {:error, reason} -> Repo.rollback({:cert_issue_failed, reason})
    end
  end

  def insert_prepared_authorship!({:attrs, attrs}) when is_map(attrs) do
    case insert_authorship_bound(attrs) do
      {:ok, cert} -> cert
      {:error, reason} -> Repo.rollback({:cert_issue_failed, reason})
    end
  end

  @doc false
  @spec insert_prepared_release!({:existing, t()} | {:attrs, signed_attrs()}) :: t()
  def insert_prepared_release!({:existing, %__MODULE__{} = cert}) do
    case recheck_release_row(cert) do
      :ok -> cert
      {:error, reason} -> Repo.rollback({:cert_issue_failed, reason})
    end
  end

  def insert_prepared_release!({:attrs, attrs}) when is_map(attrs) do
    case insert_release_bound(attrs) do
      {:ok, cert} -> cert
      {:error, reason} -> Repo.rollback({:cert_issue_failed, reason})
    end
  end

  @spec verify(String.t()) :: :ok | {:error, :tampered | :not_found | term()}
  def verify(cert_id) do
    case Repo.get(__MODULE__, cert_id) do
      nil -> {:error, :not_found}
      %__MODULE__{} = cert -> verify_stored(cert)
    end
  end

  @doc """
  Offline Authorship verify — no DB.

  `pub_key_b64` is the Ed25519 public key (raw 32 bytes, Base64) that should match
  the signing `kid`. When omitted, uses current `:cert_signing_pub_key` (see moduledoc).
  """
  @spec verify_offline_authorship(
          String.t(),
          String.t(),
          non_neg_integer(),
          String.t(),
          String.t(),
          String.t() | nil
        ) :: :ok | {:error, :tampered | term()}
  def verify_offline_authorship(
        commit_hash,
        author_sub,
        issued_at_ms,
        kid,
        signature,
        pub_key_b64 \\ nil
      )

  def verify_offline_authorship(
        commit_hash,
        author_sub,
        issued_at_ms,
        kid,
        signature,
        pub_key_b64
      )
      when is_binary(commit_hash) and is_binary(author_sub) and is_integer(issued_at_ms) and
             issued_at_ms >= 0 and is_binary(kid) and is_binary(signature) and
             (is_binary(pub_key_b64) or is_nil(pub_key_b64)) do
    with :ok <- validate_pipe_free(author_sub, :invalid_subject_sub),
         :ok <- validate_pipe_free(kid, :invalid_kid),
         {:ok, pub} <- resolve_pub_key(pub_key_b64) do
      message = authorship_message(commit_hash, author_sub, issued_at_ms, kid)
      verify_signature(message, signature, pub)
    end
  end

  def verify_offline_authorship(_, _, _, _, _, _), do: {:error, :tampered}

  @doc """
  Offline Release verify — no DB. Same `pub_key_b64` contract as Authorship.
  """
  @spec verify_offline_release(
          String.t(),
          String.t(),
          String.t(),
          String.t(),
          non_neg_integer(),
          String.t(),
          String.t(),
          String.t() | nil
        ) :: :ok | {:error, :tampered | term()}
  def verify_offline_release(
        repo_id,
        tag_name,
        commit_hash,
        signer_sub,
        issued_at_ms,
        kid,
        signature,
        pub_key_b64 \\ nil
      )

  def verify_offline_release(
        repo_id,
        tag_name,
        commit_hash,
        signer_sub,
        issued_at_ms,
        kid,
        signature,
        pub_key_b64
      ) do
    if offline_release_args_ok?(
         repo_id,
         tag_name,
         commit_hash,
         signer_sub,
         issued_at_ms,
         kid,
         signature,
         pub_key_b64
       ) do
      do_verify_offline_release(
        repo_id,
        tag_name,
        commit_hash,
        signer_sub,
        issued_at_ms,
        kid,
        signature,
        pub_key_b64
      )
    else
      {:error, :tampered}
    end
  end

  defp offline_release_args_ok?(
         repo_id,
         tag_name,
         commit_hash,
         signer_sub,
         issued_at_ms,
         kid,
         signature,
         pub_key_b64
       ) do
    offline_release_ids_ok?(repo_id, tag_name, commit_hash, signer_sub) and
      offline_release_sig_ok?(issued_at_ms, kid, signature, pub_key_b64)
  end

  defp offline_release_ids_ok?(repo_id, tag_name, commit_hash, signer_sub) do
    is_binary(repo_id) and is_binary(tag_name) and is_binary(commit_hash) and
      is_binary(signer_sub)
  end

  defp offline_release_sig_ok?(issued_at_ms, kid, signature, pub_key_b64) do
    is_integer(issued_at_ms) and issued_at_ms >= 0 and is_binary(kid) and
      is_binary(signature) and (is_binary(pub_key_b64) or is_nil(pub_key_b64))
  end

  defp do_verify_offline_release(
         repo_id,
         tag_name,
         commit_hash,
         signer_sub,
         issued_at_ms,
         kid,
         signature,
         pub_key_b64
       ) do
    with :ok <- validate_release_offline_fields(signer_sub, tag_name, repo_id, kid),
         {:ok, pub} <- resolve_pub_key(pub_key_b64) do
      message =
        release_message(repo_id, tag_name, commit_hash, signer_sub, issued_at_ms, kid)

      verify_signature(message, signature, pub)
    end
  end

  defp validate_release_offline_fields(signer_sub, tag_name, repo_id, kid) do
    with :ok <- validate_pipe_free(signer_sub, :invalid_subject_sub),
         :ok <- validate_pipe_free(tag_name, :invalid_tag_name),
         :ok <- validate_pipe_free(repo_id, :invalid_repo_id) do
      validate_pipe_free(kid, :invalid_kid)
    end
  end

  @spec authorship_message(String.t(), String.t(), non_neg_integer(), String.t()) :: binary()
  def authorship_message(commit_hash, author_sub, issued_at_ms, kid) do
    :ok = require_pipe_free!(author_sub)
    :ok = require_pipe_free!(kid)

    Enum.join(
      [@message_version, "commit", commit_hash, author_sub, Integer.to_string(issued_at_ms), kid],
      "|"
    )
  end

  @spec release_message(
          String.t(),
          String.t(),
          String.t(),
          String.t(),
          non_neg_integer(),
          String.t()
        ) :: binary()
  def release_message(repo_id, tag_name, commit_hash, signer_sub, issued_at_ms, kid) do
    :ok = require_pipe_free!(signer_sub)
    :ok = require_pipe_free!(tag_name)
    :ok = require_pipe_free!(repo_id)
    :ok = require_pipe_free!(kid)

    Enum.join(
      [
        @message_version,
        "release",
        repo_id,
        tag_name,
        commit_hash,
        signer_sub,
        Integer.to_string(issued_at_ms),
        kid
      ],
      "|"
    )
  end

  @spec install_test_key!() :: :ok
  def install_test_key! do
    unless mix_test_env?() do
      raise "Assets.Vcs.Cert.install_test_key!/0 is only allowed in Mix test"
    end

    {pub, priv} = :crypto.generate_key(:eddsa, :ed25519)
    Application.put_env(:assets, :cert_signing_priv_key, Base.encode64(priv))
    Application.put_env(:assets, :cert_signing_pub_key, Base.encode64(pub))
    Application.put_env(:assets, :cert_signing_kid, @default_kid)
    :ok
  end

  @spec current_kid() :: String.t()
  def current_kid do
    case Application.get_env(:assets, :cert_signing_kid, @default_kid) do
      kid when is_binary(kid) and kid != "" -> kid
      _ -> @default_kid
    end
  end

  @spec get_authorship(String.t()) :: t() | nil
  def get_authorship(commit_hash) when is_binary(commit_hash) do
    Repo.one(
      from(c in __MODULE__,
        where: c.cert_kind == "commit" and c.target_id == ^commit_hash
      )
    )
  end

  @spec get_release(String.t(), String.t()) :: t() | nil
  def get_release(repo_id, tag_name) when is_binary(repo_id) and is_binary(tag_name) do
    Repo.one(
      from(c in __MODULE__,
        where: c.cert_kind == "release" and c.repo_id == ^repo_id and c.tag_name == ^tag_name
      )
    )
  end

  # ---------------------------------------------------------------------------
  # Private
  # ---------------------------------------------------------------------------

  defp materialize_authorship({:existing, cert}) do
    with :ok <- recheck_authorship_row(cert), do: {:ok, cert}
  end

  defp materialize_authorship({:attrs, attrs}), do: insert_authorship_bound(attrs)

  defp materialize_release({:existing, cert}) do
    with :ok <- recheck_release_row(cert), do: {:ok, cert}
  end

  defp materialize_release({:attrs, attrs}), do: insert_release_bound(attrs)

  defp recheck_authorship_row(%__MODULE__{subject_sub: sub, target_id: hash} = cert) do
    with :ok <- assert_existing_authorship(cert, sub, hash),
         {:ok, commit} <- Vcs.get_commit(hash) do
      assert_author_match(commit, sub)
    end
  end

  defp recheck_release_row(
         %__MODULE__{
           subject_sub: sub,
           repo_id: repo_id,
           tag_name: tag_name,
           target_id: commit_hash
         } = cert
       ) do
    with :ok <- assert_existing_release(cert, sub, repo_id, tag_name, commit_hash),
         {:ok, _commit} <- Vcs.get_commit(commit_hash) do
      assert_release_binding(sub, repo_id, tag_name, commit_hash)
    end
  end

  defp sign_authorship(author_sub, commit_hash) do
    with :ok <- validate_pipe_free(author_sub, :invalid_subject_sub) do
      issued_at_ms = now_ms()
      kid = current_kid()
      message = authorship_message(commit_hash, author_sub, issued_at_ms, kid)
      {:ok, signed_attrs("commit", author_sub, commit_hash, nil, nil, issued_at_ms, kid, message)}
    end
  end

  defp sign_release(signer_sub, repo_id, tag_name, commit_hash) do
    with :ok <- validate_pipe_free(signer_sub, :invalid_subject_sub),
         :ok <- validate_pipe_free(tag_name, :invalid_tag_name),
         :ok <- validate_pipe_free(repo_id, :invalid_repo_id) do
      issued_at_ms = now_ms()
      kid = current_kid()
      message = release_message(repo_id, tag_name, commit_hash, signer_sub, issued_at_ms, kid)

      {:ok,
       signed_attrs(
         "release",
         signer_sub,
         commit_hash,
         repo_id,
         tag_name,
         issued_at_ms,
         kid,
         message
       )}
    end
  end

  defp insert_authorship_bound(attrs) do
    with :ok <- assert_authorship_attrs(attrs),
         {:ok, commit} <- Vcs.get_commit(attrs.target_id),
         :ok <- assert_author_match(commit, attrs.subject_sub) do
      insert_or_get_authorship(attrs)
    end
  end

  defp insert_release_bound(attrs) do
    with :ok <- assert_release_attrs(attrs),
         {:ok, _commit} <- Vcs.get_commit(attrs.target_id),
         :ok <-
           assert_release_binding(
             attrs.subject_sub,
             attrs.repo_id,
             attrs.tag_name,
             attrs.target_id
           ) do
      insert_or_get_release(attrs)
    end
  end

  defp assert_authorship_attrs(%{cert_kind: "commit", target_id: id, subject_sub: sub})
       when is_binary(id) and is_binary(sub),
       do: :ok

  defp assert_authorship_attrs(_), do: {:error, :invalid_cert_attrs}

  defp assert_release_attrs(%{
         cert_kind: "release",
         target_id: id,
         subject_sub: sub,
         repo_id: repo_id,
         tag_name: tag_name
       })
       when is_binary(id) and is_binary(sub) and is_binary(repo_id) and is_binary(tag_name),
       do: :ok

  defp assert_release_attrs(_), do: {:error, :invalid_cert_attrs}

  defp assert_author_match(%Commit{author_sub: author_sub}, author_sub), do: :ok
  defp assert_author_match(%Commit{}, _author_sub), do: {:error, :author_mismatch}

  defp assert_release_binding(signer_sub, repo_id, tag_name, commit_hash) do
    with {:ok, %VcsRepo{owner_sub: owner_sub}} <- get_repo(repo_id),
         :ok <- assert_owner_match(owner_sub, signer_sub),
         {:ok, %VcsRef{kind: "tag", commit_hash: ^commit_hash}} <-
           fetch_tag(repo_id, tag_name) do
      :ok
    else
      {:ok, %VcsRef{kind: "tag"}} -> {:error, :commit_mismatch}
      {:ok, %VcsRef{}} -> {:error, :not_found}
      {:error, _} = err -> err
    end
  end

  defp assert_owner_match(owner_sub, owner_sub), do: :ok
  defp assert_owner_match(_, _), do: {:error, :owner_mismatch}

  defp assert_existing_authorship(%__MODULE__{} = existing, author_sub, commit_hash) do
    if existing.subject_sub == author_sub and existing.target_id == commit_hash and
         existing.cert_kind == "commit" do
      :ok
    else
      {:error, :cert_conflict}
    end
  end

  defp assert_existing_release(
         %__MODULE__{} = existing,
         signer_sub,
         repo_id,
         tag_name,
         commit_hash
       ) do
    if existing.subject_sub == signer_sub and existing.repo_id == repo_id and
         existing.tag_name == tag_name and existing.target_id == commit_hash and
         existing.cert_kind == "release" do
      :ok
    else
      {:error, :cert_conflict}
    end
  end

  defp get_repo(repo_id) do
    case Repo.get(VcsRepo, repo_id) do
      nil -> {:error, :not_found}
      %VcsRepo{} = repo -> {:ok, repo}
    end
  end

  defp fetch_tag(repo_id, name) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: name) do
      nil -> {:error, :not_found}
      %VcsRef{} = ref -> {:ok, ref}
    end
  end

  defp insert_or_get_authorship(attrs) do
    case get_authorship(attrs.target_id) do
      %__MODULE__{} = existing ->
        take_existing_authorship(existing, attrs)

      nil ->
        insert_authorship_row(attrs)
    end
  end

  defp insert_or_get_release(attrs) do
    case get_release(attrs.repo_id, attrs.tag_name) do
      %__MODULE__{} = existing ->
        take_existing_release(existing, attrs)

      nil ->
        insert_release_row(attrs)
    end
  end

  defp take_existing_authorship(existing, attrs) do
    with :ok <- assert_existing_authorship(existing, attrs.subject_sub, attrs.target_id) do
      {:ok, existing}
    end
  end

  defp take_existing_release(existing, attrs) do
    with :ok <-
           assert_existing_release(
             existing,
             attrs.subject_sub,
             attrs.repo_id,
             attrs.tag_name,
             attrs.target_id
           ) do
      {:ok, existing}
    end
  end

  defp insert_authorship_row(attrs) do
    case Repo.insert(changeset(%__MODULE__{}, attrs)) do
      {:ok, cert} ->
        {:ok, cert}

      {:error, %Ecto.Changeset{} = cs} ->
        recover_authorship_unique(attrs, cs)
    end
  end

  defp insert_release_row(attrs) do
    case Repo.insert(changeset(%__MODULE__{}, attrs)) do
      {:ok, cert} ->
        {:ok, cert}

      {:error, %Ecto.Changeset{} = cs} ->
        recover_release_unique(attrs, cs)
    end
  end

  defp recover_authorship_unique(attrs, cs) do
    if unique_violation?(cs, :vcs_certs_authorship_commit_index) do
      case get_authorship(attrs.target_id) do
        %__MODULE__{} = existing -> take_existing_authorship(existing, attrs)
        nil -> {:error, cs}
      end
    else
      {:error, cs}
    end
  end

  defp recover_release_unique(attrs, cs) do
    if unique_violation?(cs, :vcs_certs_release_repo_tag_index) do
      case get_release(attrs.repo_id, attrs.tag_name) do
        %__MODULE__{} = existing -> take_existing_release(existing, attrs)
        nil -> {:error, cs}
      end
    else
      {:error, cs}
    end
  end

  defp unique_violation?(%Ecto.Changeset{errors: errors}, index_name) do
    Enum.any?(errors, fn
      {_field, {_msg, opts}} ->
        opts[:constraint] == :unique and opts[:constraint_name] == index_name

      _ ->
        false
    end)
  end

  defp signed_attrs(
         cert_kind,
         subject_sub,
         commit_hash,
         repo_id,
         tag_name,
         issued_at_ms,
         kid,
         message
       ) do
    priv_key = signing_priv_key!()
    target_hash = Assets.Blake3.hash_hex(message)
    sig_bin = :crypto.sign(:eddsa, :none, message, [priv_key, :ed25519])
    signature = Base.encode64(sig_bin)

    %{
      cert_id: generate_id(),
      subject_sub: subject_sub,
      cert_kind: cert_kind,
      target_id: commit_hash,
      repo_id: repo_id,
      tag_name: tag_name,
      issued_at_ms: issued_at_ms,
      kid: kid,
      message_version: @message_version,
      target_hash: target_hash,
      signature: signature
    }
  end

  defp verify_stored(%__MODULE__{
         cert_kind: "commit",
         subject_sub: subject_sub,
         target_id: commit_hash,
         issued_at_ms: issued_at_ms,
         kid: kid,
         message_version: @message_version,
         target_hash: stored_hash,
         signature: signature
       }) do
    message = authorship_message(commit_hash, subject_sub, issued_at_ms, kid)
    check_hash_and_sig(message, stored_hash, signature)
  end

  defp verify_stored(%__MODULE__{
         cert_kind: "release",
         subject_sub: subject_sub,
         target_id: commit_hash,
         repo_id: repo_id,
         tag_name: tag_name,
         issued_at_ms: issued_at_ms,
         kid: kid,
         message_version: @message_version,
         target_hash: stored_hash,
         signature: signature
       })
       when is_binary(repo_id) and is_binary(tag_name) do
    message = release_message(repo_id, tag_name, commit_hash, subject_sub, issued_at_ms, kid)
    check_hash_and_sig(message, stored_hash, signature)
  end

  defp verify_stored(_), do: {:error, :tampered}

  defp check_hash_and_sig(message, stored_hash, signature) do
    expected_hash = Assets.Blake3.hash_hex(message)

    if expected_hash != stored_hash do
      {:error, :tampered}
    else
      with {:ok, pub} <- resolve_pub_key(nil) do
        verify_signature(message, signature, pub)
      end
    end
  end

  defp verify_signature(message, signature, pub_key) do
    case Base.decode64(signature) do
      :error ->
        {:error, :tampered}

      {:ok, sig_bin} ->
        if :crypto.verify(:eddsa, :none, message, sig_bin, [pub_key, :ed25519]) do
          :ok
        else
          {:error, :tampered}
        end
    end
  end

  defp resolve_pub_key(nil), do: {:ok, signing_pub_key!()}

  defp resolve_pub_key(b64) when is_binary(b64) do
    case Base.decode64(b64) do
      {:ok, pub} when byte_size(pub) == 32 -> {:ok, pub}
      _ -> {:error, :tampered}
    end
  end

  defp validate_pipe_free(value, error_atom) when is_binary(value) do
    if String.contains?(value, "|"), do: {:error, error_atom}, else: :ok
  end

  defp validate_pipe_free(_, error_atom), do: {:error, error_atom}

  defp require_pipe_free!(value) do
    case validate_pipe_free(value, :invalid) do
      :ok ->
        :ok

      {:error, _} ->
        raise ArgumentError,
              "Origin Cert field must be a binary without \"|\" (pipe field separator)"
    end
  end

  defp now_ms do
    if mix_test_env?() do
      case Application.get_env(:assets, :vcs_test_issued_at_ms) do
        ms when is_integer(ms) and ms >= 0 -> ms
        _ -> System.system_time(:millisecond)
      end
    else
      System.system_time(:millisecond)
    end
  end

  defp mix_test_env? do
    Code.ensure_loaded?(Mix) and function_exported?(Mix, :env, 0) and Mix.env() == :test
  end

  defp signing_priv_key! do
    case Application.get_env(:assets, :cert_signing_priv_key) do
      nil -> raise "Assets :cert_signing_priv_key not configured"
      b64 -> Base.decode64!(b64)
    end
  end

  defp signing_pub_key! do
    case Application.get_env(:assets, :cert_signing_pub_key) do
      nil -> raise "Assets :cert_signing_pub_key not configured"
      b64 -> Base.decode64!(b64)
    end
  end

  defp generate_id do
    :crypto.strong_rand_bytes(16)
    |> Base.encode16(case: :lower)
    |> then(fn hex ->
      <<a::binary-8, b::binary-4, c::binary-4, d::binary-4, e::binary-12>> = hex
      "#{a}-#{b}-#{c}-#{d}-#{e}"
    end)
  end
end
