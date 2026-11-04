defmodule Assets.Vcs.Cert do
  @moduledoc """
  VcsCert — Origin Certificate thin peel (x0065).

  Signs and verifies provenance of a VcsCommit or VcsRef(tag) using Ed25519
  via OTP `:crypto`. The private key NEVER appears in responses, logs, or changesets.

  Key management:
  - Config key: `Application.get_env(:assets, :cert_signing_priv_key)` — Base64 raw Ed25519 private key.
  - Config key: `Application.get_env(:assets, :cert_signing_pub_key)` — Base64 raw Ed25519 public key.
  - In test: call `Assets.Vcs.Cert.install_test_key!/0` in setup.
  - Production: inject via runtime config / secrets manager; never commit to VCS.

  Canonical message (what is signed):
    `"target_kind:target_id:target_hash:subject_sub"` (fields joined by `:`)

  Guarantees:
  - issue/2 rejects unknown target_kind.
  - verify/1 returns {:error, :tampered} for any mismatch.
  - Private key is never returned or logged.

  Non-guarantees: CA chain, revocation, multi-sig, License Token.
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias Assets.Repo

  @target_kind_values ~w(commit tag)

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
  Issue a cert. Signs `canonical_message/3` with the configured Ed25519 key.
  Returns {:ok, cert} on success.
  """
  @type t :: %__MODULE__{}

  @spec issue(String.t(), String.t(), String.t()) ::
          {:ok, t()} | {:error, term()}
  def issue(subject_sub, target_kind, target_id)
      when target_kind in @target_kind_values do
    priv_key = signing_priv_key!()
    message = canonical_message(target_kind, target_id, subject_sub)
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

  def issue(_subject_sub, target_kind, _target_id) do
    {:error, {:invalid_target_kind, target_kind}}
  end

  @doc """
  Verify a stored cert. Checks:
  1. target_hash matches recomputed blake3 of canonical message.
  2. Ed25519 signature over canonical message is valid.
  Returns :ok or {:error, :tampered | :not_found | term()}.
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
    message = canonical_message(target_kind, target_id, subject_sub)
    expected_hash = Assets.Blake3.hash_hex(message)

    if expected_hash != stored_hash do
      {:error, :tampered}
    else
      sig_bin = Base.decode64!(signature)
      pub_key = signing_pub_key!()

      if :crypto.verify(:eddsa, :none, message, sig_bin, [pub_key, :ed25519]) do
        :ok
      else
        {:error, :tampered}
      end
    end
  end

  @doc """
  Generate a fresh Ed25519 keypair and install it in Application config.
  Use in test setup only — never in production code.
  """
  @spec install_test_key!() :: :ok
  def install_test_key! do
    {pub, priv} = :crypto.generate_key(:eddsa, :ed25519)
    Application.put_env(:assets, :cert_signing_priv_key, Base.encode64(priv))
    Application.put_env(:assets, :cert_signing_pub_key, Base.encode64(pub))
    :ok
  end

  @doc "Canonical message that is signed. Public so callers can verify external certs."
  @spec canonical_message(String.t(), String.t(), String.t()) :: binary()
  def canonical_message(target_kind, target_id, subject_sub) do
    "#{target_kind}:#{target_id}:#{subject_sub}"
  end

  # ---------------------------------------------------------------------------
  # Private
  # ---------------------------------------------------------------------------

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
