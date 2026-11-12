defmodule Assets.Vcs.Cert do
  @moduledoc """
  VcsCert — Origin Certificate (x0065 thin peel + x0096 content-bound v1).

  Signs and verifies provenance of a VcsCommit or VcsRef(tag) using Ed25519
  via OTP `:crypto`. The private key NEVER appears in responses, logs, or changesets.

  Key management:
  - Config key: `Application.get_env(:assets, :cert_signing_priv_key)` — Base64 raw Ed25519 private key.
  - Config key: `Application.get_env(:assets, :cert_signing_pub_key)` — Base64 raw Ed25519 public key.
  - In test: call `Assets.Vcs.Cert.install_test_key!/0` in setup.
  - Production: inject via runtime config / secrets manager; never commit to VCS.

  Canonical message v1 (what is signed) — pipe-separated, fixed before federation:

      `v1|{target_kind}|{target_id}|{subject_sub}|{tree_blake3}|{parent_commit_id}`

  - `tree_blake3` = BLAKE3 hex of the target Commit's `tree_json` (content binding).
  - `parent_commit_id` = Commit.parent_commit_id, or empty string for a root commit.
  - Tag targets resolve through the tagged Commit (same tree/parent fields).
  - `target_hash` on the cert row is blake3 of that message (tamper anchor), not a signed field.
  - Fields are `|`-separated. `subject_sub` must not contain `|` (rejected on issue) so
    external split-based verifiers keep stable field boundaries. `target_id` / `tree_blake3`
    / parent are UUID or hex in practice. In-process verify rebuilds from structured args.

  Error surface (intentional asymmetry):
  - issue: missing Commit/Tag → `{:error, :not_found}` (nothing to sign).
  - verify: cert row missing → `{:error, :not_found}`; target Commit/Tag gone after issue,
    or tree/parent/signature mismatch → `{:error, :tampered}` (fail-closed: cert exists but
    content binding can no longer be proven; GC vs rewrite are not distinguished).

  Migration: pre-x0096 rows signed as `kind:id:sub` will not verify under v1 — re-issue.

  Guarantees:
  - issue/3 rejects unknown target_kind, missing commit/tag, and `|` in subject_sub.
  - verify/1 returns {:error, :tampered} for content/parent/signature mismatch or missing
    target after issue; {:error, :not_found} only when the cert row itself is absent.
  - Private key is never returned or logged.

  Non-guarantees: CA chain, revocation, multi-sig, License Token, offline export schema,
    length-prefix encoding (pipe + sub charset constraint instead).
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias Assets.Repo
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Ref, as: VcsRef

  @target_kind_values ~w(commit tag)
  @message_version "v1"

  @primary_key {:cert_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_certs" do
    field :subject_sub, :string
    field :target_kind, :string
    field :target_id, :string
    field :target_hash, :string
    field :signature, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(cert, attrs) do
    cert
    |> cast(attrs, [:cert_id, :subject_sub, :target_kind, :target_id, :target_hash, :signature])
    |> validate_required([
      :cert_id,
      :subject_sub,
      :target_kind,
      :target_id,
      :target_hash,
      :signature
    ])
    |> validate_inclusion(:target_kind, @target_kind_values)
  end

  # ---------------------------------------------------------------------------
  # Sign / verify
  # ---------------------------------------------------------------------------

  @doc """
  Issue a cert. Resolves Commit (or Tag→Commit), signs `canonical_message/5`.
  Returns {:ok, cert} on success.
  """
  @type t :: %__MODULE__{}

  @spec issue(String.t(), String.t(), String.t()) ::
          {:ok, t()} | {:error, term()}
  def issue(subject_sub, target_kind, target_id)
      when is_binary(subject_sub) and target_kind in @target_kind_values do
    with :ok <- validate_subject_sub(subject_sub),
         {:ok, commit} <- resolve_commit(target_kind, target_id) do
      priv_key = signing_priv_key!()
      tree_blake3 = tree_blake3(commit)
      parent = parent_field(commit)
      message = canonical_message(target_kind, target_id, subject_sub, tree_blake3, parent)
      # target_hash is the blake3 of the canonical message (tamper anchor)
      target_hash = Assets.Blake3.hash_hex(message)
      # Ed25519 signature: :crypto.sign(:eddsa, :none, Msg, [PrivKey, :ed25519])
      sig_bin = :crypto.sign(:eddsa, :none, message, [priv_key, :ed25519])
      signature = Base.encode64(sig_bin)

      cert_id = generate_id()

      changeset =
        changeset(%__MODULE__{}, %{
          cert_id: cert_id,
          subject_sub: subject_sub,
          target_kind: target_kind,
          target_id: target_id,
          target_hash: target_hash,
          signature: signature
        })

      Repo.insert(changeset)
    end
  end

  def issue(subject_sub, target_kind, _target_id)
      when target_kind in @target_kind_values and not is_binary(subject_sub) do
    {:error, :invalid_subject_sub}
  end

  def issue(_subject_sub, target_kind, _target_id) do
    {:error, {:invalid_target_kind, target_kind}}
  end

  @doc """
  Verify a stored cert. Checks:
  1. Target Commit/Tag still resolves (missing → :tampered, fail-closed).
  2. target_hash matches recomputed blake3 of canonical message (incl. current tree/parent).
  3. Ed25519 signature over that message is valid.
  Returns :ok or {:error, :tampered | :not_found | term()}.
  `:not_found` is only for a missing cert row.
  """
  @spec verify(String.t()) :: :ok | {:error, :tampered | :not_found | term()}
  def verify(cert_id) do
    case Repo.get(__MODULE__, cert_id) do
      nil ->
        {:error, :not_found}

      %__MODULE__{} = cert ->
        verify_cert(cert)
    end
  end

  defp verify_cert(%__MODULE__{
         subject_sub: subject_sub,
         target_kind: target_kind,
         target_id: target_id,
         target_hash: stored_hash,
         signature: signature
       }) do
    case resolve_commit(target_kind, target_id) do
      # Target gone (GC / delete): cert row remains but content binding cannot be proven.
      {:error, :not_found} ->
        {:error, :tampered}

      {:ok, commit} ->
        tree_blake3 = tree_blake3(commit)
        parent = parent_field(commit)
        message = canonical_message(target_kind, target_id, subject_sub, tree_blake3, parent)
        expected_hash = Assets.Blake3.hash_hex(message)

        if expected_hash != stored_hash do
          {:error, :tampered}
        else
          verify_signature(message, signature)
        end
    end
  end

  defp verify_signature(message, signature) do
    case Base.decode64(signature) do
      :error ->
        {:error, :tampered}

      {:ok, sig_bin} ->
        if :crypto.verify(:eddsa, :none, message, sig_bin, [signing_pub_key!(), :ed25519]) do
          :ok
        else
          {:error, :tampered}
        end
    end
  end

  @doc """
  Generate a fresh Ed25519 keypair and install it in Application config.
  Test-only — raises outside `Mix.env() == :test`.
  """
  @spec install_test_key!() :: :ok
  def install_test_key! do
    unless mix_test_env?() do
      raise "Assets.Vcs.Cert.install_test_key!/0 is only allowed in Mix test"
    end

    {pub, priv} = :crypto.generate_key(:eddsa, :ed25519)
    Application.put_env(:assets, :cert_signing_priv_key, Base.encode64(priv))
    Application.put_env(:assets, :cert_signing_pub_key, Base.encode64(pub))
    :ok
  end

  defp mix_test_env? do
    Code.ensure_loaded?(Mix) and function_exported?(Mix, :env, 0) and Mix.env() == :test
  end

  @doc """
  Canonical v1 message that is signed.
  Public so callers can verify external certs with the same payload shape.
  `parent_commit_id` may be nil (encoded as empty field).

  Enforces the same `subject_sub` charset as `issue/3`: must be a binary without `|`.
  Raises `ArgumentError` otherwise — do not bypass; split-based external verifiers
  rely on this field-boundary guarantee.
  """
  @spec canonical_message(String.t(), String.t(), String.t(), String.t(), String.t() | nil) ::
          binary()
  def canonical_message(target_kind, target_id, subject_sub, tree_blake3, parent_commit_id) do
    :ok = require_subject_sub!(subject_sub)
    parent = parent_commit_id || ""

    Enum.join(
      [
        @message_version,
        target_kind,
        target_id,
        subject_sub,
        tree_blake3,
        parent
      ],
      "|"
    )
  end

  @doc "BLAKE3 hex of a Commit's tree_json (content hash bound into the cert)."
  @spec tree_blake3(Commit.t()) :: String.t()
  def tree_blake3(%Commit{tree_json: tree_json}) when is_binary(tree_json) do
    Assets.Blake3.hash_hex(tree_json)
  end

  # ---------------------------------------------------------------------------
  # Private
  # ---------------------------------------------------------------------------

  defp validate_subject_sub(subject_sub) when is_binary(subject_sub) do
    if String.contains?(subject_sub, "|") do
      {:error, :invalid_subject_sub}
    else
      :ok
    end
  end

  defp validate_subject_sub(_), do: {:error, :invalid_subject_sub}

  defp require_subject_sub!(subject_sub) do
    case validate_subject_sub(subject_sub) do
      :ok ->
        :ok

      {:error, :invalid_subject_sub} ->
        raise ArgumentError,
              "Origin Cert subject_sub must be a binary without \"|\" (pipe field separator)"
    end
  end

  defp resolve_commit("commit", commit_id) do
    case Repo.get(Commit, commit_id) do
      nil -> {:error, :not_found}
      %Commit{} = commit -> {:ok, commit}
    end
  end

  defp resolve_commit("tag", ref_id) do
    case Repo.get(VcsRef, ref_id) do
      %VcsRef{kind: "tag", commit_id: commit_id} ->
        resolve_commit("commit", commit_id)

      _ ->
        {:error, :not_found}
    end
  end

  defp parent_field(%Commit{parent_commit_id: nil}), do: ""
  defp parent_field(%Commit{parent_commit_id: parent}) when is_binary(parent), do: parent

  # Private key — NEVER log or return this value.
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
