defmodule Assets.Vcs.Cert do
  @moduledoc """
  VcsCert — Origin Certificate (x0065 / x0096 v1 + x00c9 content-addressed IDs).

  Canonical message v1:

      `v1|{target_kind}|{target_id}|{subject_sub}|{tree_blake3}|{parent_commit_id}`

  - `target_id` = commit_hash (or tag ref_id).
  - `tree_blake3` = Tree **object** hash (`Commit.tree_hash`), not Prefab JSON.
  - `parent_commit_id` = first parent hash, or empty for root.

  Guarantees: issue/verify fail-closed; private key never logged.
  Non-guarantees: Cert v2 (`issued_at_ms` / `kid`); CA chain.
  """

  use Ecto.Schema
  import Ecto.Changeset

  alias Assets.Repo
  alias Assets.Vcs
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
      target_hash = Assets.Blake3.hash_hex(message)
      sig_bin = :crypto.sign(:eddsa, :none, message, [priv_key, :ed25519])
      signature = Base.encode64(sig_bin)

      changeset =
        changeset(%__MODULE__{}, %{
          cert_id: generate_id(),
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

  @spec verify(String.t()) :: :ok | {:error, :tampered | :not_found | term()}
  def verify(cert_id) do
    case Repo.get(__MODULE__, cert_id) do
      nil -> {:error, :not_found}
      %__MODULE__{} = cert -> verify_cert(cert)
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

  @spec canonical_message(String.t(), String.t(), String.t(), String.t(), String.t() | nil) ::
          binary()
  def canonical_message(target_kind, target_id, subject_sub, tree_blake3, parent_commit_id) do
    :ok = require_subject_sub!(subject_sub)
    parent = parent_commit_id || ""

    Enum.join(
      [@message_version, target_kind, target_id, subject_sub, tree_blake3, parent],
      "|"
    )
  end

  @doc "Tree object hash bound into the cert (x00c9)."
  @spec tree_blake3(Commit.t()) :: String.t()
  def tree_blake3(%Commit{tree_hash: tree_hash}) when is_binary(tree_hash), do: tree_hash

  defp validate_subject_sub(subject_sub) when is_binary(subject_sub) do
    if String.contains?(subject_sub, "|"), do: {:error, :invalid_subject_sub}, else: :ok
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

  defp resolve_commit("commit", commit_hash) do
    Vcs.get_commit(commit_hash)
  end

  defp resolve_commit("tag", ref_id) do
    case Repo.get(VcsRef, ref_id) do
      %VcsRef{kind: "tag", commit_hash: commit_hash} ->
        resolve_commit("commit", commit_hash)

      _ ->
        {:error, :not_found}
    end
  end

  defp parent_field(%Commit{commit_hash: hash}) do
    Vcs.first_parent_hash(hash) || ""
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
