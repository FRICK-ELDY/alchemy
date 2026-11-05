defmodule Assets.Drive do
  @moduledoc """
  Personal Drive CRUD (x005c folder/blob, x005d prefab kind, x005f take-out, x0060 put-back,
  x006e HTTP put_back).

  Guarantees:
  - Owner-only CRUD for DriveNode (folder / blob / prefab).
  - blob/prefab creation requires owner to hold a BlobGrant for the referenced blake3.
  - parent_id must point to an owner-controlled folder (not another kind).
  - Circular parent chain is rejected (bounded hop walk).
  - resolve_for_take_out/2 verifies grant ownership before host spawn (x005f).
  - save_prefab/4 validates Prefab.Payload schema and parent authz before upload (x0060).

  Non-guarantees:
  - Group Vault, sharing, Mail/Comms, Shop, VcsRepo ops (→ Assets.Vcs).
  - Compensating delete when upload succeeds but DriveNode upsert fails (CAS bytes +
    quota/Grant may remain; CAS orphan bytes without CasObject are handled by
    Assets.CAS.OrphanGc (x0074), not by Drive delete).
  """

  import Ecto.Query

  alias Assets.Blobs
  alias Assets.Blobs.BlobGrant
  alias Assets.Drive.Node
  alias Assets.Prefab.Payload
  alias Assets.Repo

  @max_cycle_depth 32

  # ---------------------------------------------------------------------------
  # Folder
  # ---------------------------------------------------------------------------

  @doc "Create a folder (optionally nested under an owner-controlled parent folder)."
  @spec create_folder(String.t(), String.t(), String.t() | nil) ::
          {:ok, Node.t()}
          | {:error, Ecto.Changeset.t() | :forbidden | :parent_not_folder | term()}
  def create_folder(owner_sub, name, parent_id \\ nil) do
    with :ok <- maybe_assert_parent_owner(owner_sub, parent_id),
         :ok <- check_no_cycle(owner_sub, parent_id) do
      node_id = generate_id()

      changeset =
        Node.changeset(%Node{}, %{
          node_id: node_id,
          owner_sub: owner_sub,
          parent_id: parent_id,
          kind: "folder",
          name: name
        })

      Repo.insert(changeset)
    end
  end

  # ---------------------------------------------------------------------------
  # Blob leaf (x005c)
  # ---------------------------------------------------------------------------

  @doc """
  Create a blob leaf pointing at an existing CasObject.
  Owner must already hold a BlobGrant for the referenced blake3 (x004f).
  """
  @spec create_blob(String.t(), String.t(), String.t(), String.t() | nil) ::
          {:ok, Node.t()} | {:error, Ecto.Changeset.t() | :forbidden | :no_grant | term()}
  def create_blob(owner_sub, name, blake3, parent_id \\ nil) do
    with :ok <- maybe_assert_parent_owner(owner_sub, parent_id),
         :ok <- assert_blob_grant(owner_sub, blake3) do
      changeset =
        Node.changeset(%Node{}, %{
          node_id: generate_id(),
          owner_sub: owner_sub,
          parent_id: parent_id,
          kind: "blob",
          name: name,
          blake3: blake3
        })

      Repo.insert(changeset)
    end
  end

  # ---------------------------------------------------------------------------
  # Prefab leaf (x005d)
  # ---------------------------------------------------------------------------

  @doc """
  Create a prefab leaf pointing at an already-uploaded Prefab payload blob.
  Owner must already hold a BlobGrant for the payload blake3.
  Use save_prefab/4 to upload-and-link in one call (x0060).
  """
  @spec create_prefab_node(String.t(), String.t(), String.t(), String.t() | nil) ::
          {:ok, Node.t()} | {:error, Ecto.Changeset.t() | :forbidden | :no_grant | term()}
  def create_prefab_node(owner_sub, name, payload_blake3, parent_id \\ nil) do
    with :ok <- maybe_assert_parent_owner(owner_sub, parent_id),
         :ok <- assert_blob_grant(owner_sub, payload_blake3) do
      changeset =
        Node.changeset(%Node{}, %{
          node_id: generate_id(),
          owner_sub: owner_sub,
          parent_id: parent_id,
          kind: "prefab",
          name: name,
          blake3: payload_blake3
        })

      Repo.insert(changeset)
    end
  end

  # ---------------------------------------------------------------------------
  # Read / List
  # ---------------------------------------------------------------------------

  @doc "List direct children of parent_id (or root when nil), owner-only."
  @spec list_children(String.t(), String.t() | nil) :: [Node.t()]
  def list_children(owner_sub, parent_id \\ nil) do
    query =
      if is_nil(parent_id) do
        from(n in Node,
          where: n.owner_sub == ^owner_sub and is_nil(n.parent_id),
          order_by: [asc: n.inserted_at]
        )
      else
        from(n in Node,
          where: n.owner_sub == ^owner_sub and n.parent_id == ^parent_id,
          order_by: [asc: n.inserted_at]
        )
      end

    Repo.all(query)
  end

  @doc "Get a node by id — returns nil if not found or owner mismatch."
  @spec get_node(String.t(), String.t()) :: Node.t() | nil
  def get_node(owner_sub, node_id) do
    Repo.get_by(Node, node_id: node_id, owner_sub: owner_sub)
  end

  # ---------------------------------------------------------------------------
  # Delete
  # ---------------------------------------------------------------------------

  @doc "Delete a node (must be owner; folder must be empty first)."
  @spec delete_node(String.t(), String.t()) ::
          :ok | {:error, :not_found | :not_empty | Ecto.Changeset.t()}
  def delete_node(owner_sub, node_id) do
    case get_node(owner_sub, node_id) do
      nil ->
        {:error, :not_found}

      %Node{kind: "folder"} = node ->
        if folder_has_children?(owner_sub, node_id) do
          {:error, :not_empty}
        else
          delete_ok(node)
        end

      %Node{} = node ->
        delete_ok(node)
    end
  end

  defp delete_ok(%Node{} = node) do
    case Repo.delete(node) do
      {:ok, _} -> :ok
      {:error, changeset} -> {:error, changeset}
    end
  end

  # ---------------------------------------------------------------------------
  # Take-out (x005f)
  # ---------------------------------------------------------------------------

  @doc """
  Resolve a DriveNode for inventory take-out (x005f).

  Verifies the owner holds a BlobGrant for every blob referenced by the node.
  Returns the node and the list of blake3 refs the host may use.

  Host receives blake3 refs only — NOT Drive node_id (Drive IDs are private
  persistent identifiers; host works only with ItemRef / entity_id at runtime).
  """
  @spec resolve_for_take_out(String.t(), String.t()) ::
          {:ok, %{node: Node.t(), blobs: [%{blake3: String.t()}]}}
          | {:error, :not_found | :no_grant}
  def resolve_for_take_out(owner_sub, node_id) do
    case get_node(owner_sub, node_id) do
      nil ->
        {:error, :not_found}

      %Node{} = node ->
        with :ok <- assert_all_grants(owner_sub, node) do
          {:ok, %{node: node, blobs: collect_blobs(node)}}
        end
    end
  end

  # ---------------------------------------------------------------------------
  # Put-back / save Prefab (x0060)
  # ---------------------------------------------------------------------------

  @doc """
  Save a Prefab payload as a DriveNode (x0060 put-back).

  Steps:
  1. Validate payload via Assets.Prefab.Payload (rejects non-finite TRS, bad schema).
  2. Upload payload JSON via Assets.Blobs (Quota + owned BlobGrant; x006f).
  3. Upsert DriveNode(kind=prefab, name=name, blake3=new_blake3).
     If a prefab with the same owner+name+parent already exists, its blake3 is updated.

  Returns {:ok, node} on success.
  """
  @spec save_prefab(String.t(), String.t(), Payload.t(), String.t() | nil) ::
          {:ok, Node.t()} | {:error, term()}
  def save_prefab(owner_sub, name, %Payload{} = payload, parent_id \\ nil) do
    # Parent/authz checks before upload so :forbidden does not consume quota or orphan CAS.
    with :ok <- maybe_assert_parent_owner(owner_sub, parent_id),
         {:ok, json} <- Payload.to_json(payload),
         {:ok, cas_obj, _status} <- Blobs.upload(owner_sub, json, "application/json") do
      upsert_prefab_node(owner_sub, name, cas_obj.blake3, parent_id)
    end
  end

  # ---------------------------------------------------------------------------
  # Private helpers
  # ---------------------------------------------------------------------------

  defp maybe_assert_parent_owner(_owner_sub, nil), do: :ok

  defp maybe_assert_parent_owner(owner_sub, parent_id) do
    case Repo.get_by(Node, node_id: parent_id) do
      nil -> {:error, :forbidden}
      %Node{owner_sub: ^owner_sub, kind: "folder"} -> :ok
      %Node{owner_sub: ^owner_sub} -> {:error, :parent_not_folder}
      %Node{} -> {:error, :forbidden}
    end
  end

  # Rejects a new node whose parent chain contains owner_sub's proposed node.
  # Called after maybe_assert_parent_owner, so parent exists and is owned.
  defp check_no_cycle(_owner_sub, nil), do: :ok

  defp check_no_cycle(owner_sub, parent_id) do
    # Walk up from parent_id at most @max_cycle_depth times.
    # A cycle cannot exist if we reach nil or a node without a parent.
    # (New nodes have no children yet, so cycles can only form if we're pointing
    # at our own ancestor — not possible for a brand-new node_id.)
    do_cycle_check(owner_sub, parent_id, @max_cycle_depth)
  end

  defp do_cycle_check(_owner_sub, nil, _hops), do: :ok
  defp do_cycle_check(_owner_sub, _id, 0), do: {:error, :cycle_depth_exceeded}

  defp do_cycle_check(owner_sub, node_id, hops) do
    case Repo.get_by(Node, node_id: node_id, owner_sub: owner_sub) do
      nil -> :ok
      %Node{parent_id: nil} -> :ok
      %Node{parent_id: pid} -> do_cycle_check(owner_sub, pid, hops - 1)
    end
  end

  defp assert_blob_grant(owner_sub, blake3) do
    case Repo.get_by(BlobGrant, grantee_sub: owner_sub, blake3: blake3) do
      %BlobGrant{} -> :ok
      nil -> {:error, :no_grant}
    end
  end

  defp assert_all_grants(owner_sub, %Node{kind: kind, blake3: blake3})
       when kind in ["blob", "prefab"] do
    assert_blob_grant(owner_sub, blake3)
  end

  defp assert_all_grants(_owner_sub, %Node{}), do: :ok

  defp collect_blobs(%Node{kind: kind, blake3: blake3}) when kind in ["blob", "prefab"],
    do: [%{blake3: blake3}]

  defp collect_blobs(%Node{}), do: []

  defp folder_has_children?(owner_sub, parent_id) do
    Repo.exists?(from(n in Node, where: n.owner_sub == ^owner_sub and n.parent_id == ^parent_id))
  end

  defp upsert_prefab_node(owner_sub, name, blake3, parent_id) do
    case find_prefab_node(owner_sub, name, parent_id) do
      nil ->
        insert_or_update_prefab(owner_sub, name, blake3, parent_id)

      existing_node ->
        Repo.update(Node.changeset(existing_node, %{blake3: blake3}))
    end
  end

  defp insert_or_update_prefab(owner_sub, name, blake3, parent_id) do
    changeset =
      Node.changeset(%Node{}, %{
        node_id: generate_id(),
        owner_sub: owner_sub,
        parent_id: parent_id,
        kind: "prefab",
        name: name,
        blake3: blake3
      })

    try do
      case Repo.insert(changeset) do
        {:ok, node} ->
          {:ok, node}

        {:error, %Ecto.Changeset{} = cs} ->
          recover_prefab_unique_conflict(cs, owner_sub, name, blake3, parent_id)
      end
    rescue
      # ecto_sqlite3 may raise when unique_constraint name does not match adapter report.
      e in Ecto.ConstraintError ->
        if prefab_unique_constraint_error?(e) do
          update_existing_prefab({:error, e}, owner_sub, name, blake3, parent_id)
        else
          reraise e, __STACKTRACE__
        end
    end
  end

  # Race: concurrent insert hit unique (owner,name,parent) — reload and update.
  defp recover_prefab_unique_conflict(cs, owner_sub, name, blake3, parent_id) do
    if unique_name_conflict?(cs) do
      update_existing_prefab(cs, owner_sub, name, blake3, parent_id)
    else
      {:error, cs}
    end
  end

  defp update_existing_prefab(err, owner_sub, name, blake3, parent_id) do
    case find_prefab_node(owner_sub, name, parent_id) do
      nil -> err
      existing -> Repo.update(Node.changeset(existing, %{blake3: blake3}))
    end
  end

  # Accept any :unique on the changeset — name strings differ across adapters
  # (ecto_sqlite3 has reported `…_index` vs migration `…_unique` elsewhere).
  defp unique_name_conflict?(%Ecto.Changeset{errors: errors}) do
    Enum.any?(errors, fn
      {_field, {_, opts}} -> opts[:constraint] == :unique
      _ -> false
    end)
  end

  defp prefab_unique_constraint_error?(%Ecto.ConstraintError{constraint: name, type: type}) do
    type == :unique and prefab_unique_name?(name)
  end

  defp prefab_unique_name?(name) when is_atom(name), do: prefab_unique_name?(Atom.to_string(name))

  defp prefab_unique_name?(name) when is_binary(name) do
    name in [
      "drive_nodes_prefab_owner_name_parent",
      "drive_nodes_prefab_owner_name_root",
      "drive_nodes_owner_sub_name_parent_id_index",
      "drive_nodes_owner_sub_name_index"
    ] or String.contains?(name, "prefab_owner_name") or
      String.contains?(name, "owner_sub_name")
  end

  defp find_prefab_node(owner_sub, name, parent_id) do
    query =
      if is_nil(parent_id) do
        from(n in Node,
          where:
            n.owner_sub == ^owner_sub and n.kind == "prefab" and n.name == ^name and
              is_nil(n.parent_id)
        )
      else
        from(n in Node,
          where:
            n.owner_sub == ^owner_sub and n.kind == "prefab" and n.name == ^name and
              n.parent_id == ^parent_id
        )
      end

    Repo.one(query)
  end

  defp generate_id do
    # UUID v4 as lowercase hex string (matches drive_nodes PK type :string)
    :crypto.strong_rand_bytes(16)
    |> Base.encode16(case: :lower)
    |> then(fn hex ->
      # Format as canonical UUID
      <<a::binary-8, b::binary-4, c::binary-4, d::binary-4, e::binary-12>> = hex
      "#{a}-#{b}-#{c}-#{d}-#{e}"
    end)
  end
end
