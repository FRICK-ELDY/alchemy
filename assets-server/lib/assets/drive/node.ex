defmodule Assets.Drive.Node do
  @moduledoc """
  Personal Drive node (x005c DriveNode folder/blob, x005d kind=prefab).

  Guarantees:
  - node_id stable UUID, owner_sub, parent_id nullable, kind ∈ {folder,blob,prefab,vcs_repo}
  - blob/prefab require blake3 (64-char hex); folder must not have blake3
  - vcs_repo_id nullable — upgrade path for x0062 Prefab→VcsRepo promotion

  Non-guarantees: vcs_repo_id active use (x0062+), Group Vault, Mail/Comms, Shop.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @kind_values ~w(folder blob prefab vcs_repo)

  @primary_key {:node_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "drive_nodes" do
    field :owner_sub, :string
    # nullable self-reference (application invariant — no FK constraint)
    field :parent_id, :string
    field :kind, :string
    field :name, :string
    # blob/prefab payload blake3 (64-char hex); null for folder
    field :blake3, :string
    # future VcsRepo link (x0062+); null until promoted
    field :vcs_repo_id, :string

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(node, attrs) do
    node
    |> cast(attrs, [:node_id, :owner_sub, :parent_id, :kind, :name, :blake3, :vcs_repo_id])
    |> validate_required([:node_id, :owner_sub, :kind, :name])
    |> validate_inclusion(:kind, @kind_values)
    |> validate_length(:name, min: 1, max: 255)
    |> validate_blake3_for_kind()
    # Migration index names + ecto_sqlite3 reported names (column-derived `…_index`).
    |> unique_constraint(:name, name: :drive_nodes_prefab_owner_name_parent)
    |> unique_constraint(:name, name: :drive_nodes_prefab_owner_name_root)
    |> unique_constraint(:name, name: :drive_nodes_owner_sub_name_parent_id_index)
    |> unique_constraint(:name, name: :drive_nodes_owner_sub_name_index)
  end

  defp validate_blake3_for_kind(changeset) do
    kind = get_field(changeset, :kind)
    blake3 = get_field(changeset, :blake3)

    changeset
    |> validate_blake3_required(kind, blake3)
    |> validate_blake3_length(kind, blake3)
    |> validate_blake3_absent_for_folder(kind, blake3)
  end

  defp validate_blake3_required(cs, kind, blake3) when kind in ["blob", "prefab"] do
    if is_nil(blake3) or blake3 == "" do
      add_error(cs, :blake3, "is required for #{kind} kind")
    else
      cs
    end
  end

  defp validate_blake3_required(cs, _kind, _blake3), do: cs

  defp validate_blake3_length(cs, kind, blake3) when kind in ["blob", "prefab"] do
    if not is_nil(blake3) and byte_size(blake3) != 64 do
      add_error(cs, :blake3, "must be 64-char hex")
    else
      cs
    end
  end

  defp validate_blake3_length(cs, _kind, _blake3), do: cs

  defp validate_blake3_absent_for_folder(cs, "folder", nil), do: cs

  defp validate_blake3_absent_for_folder(cs, "folder", _blake3) do
    add_error(cs, :blake3, "must be nil for folder kind")
  end

  defp validate_blake3_absent_for_folder(cs, _kind, _blake3), do: cs
end
