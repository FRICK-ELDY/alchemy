defmodule Assets.Vcs.Push do
  @moduledoc """
  Negotiate + push accept path (x00cc / draft §7.1·§18.5).

  Guarantees: owner-only; Port verify; author_sub match; child owned Grant + CAS
  (prepare + TX 内再検証); ref CAS + FF (generation-bounded); protected →
  `:ref_protected`; single TX write of tip-reachable objects / commits /
  repo_commits / repo_blobs / origins / Authorship / ref. joining ≤256;
  tree child resolution is fail-closed; orphan trees are not stored.

  Non-guarantees: fork push; `:purchased` / `:imported`; clone/fetch; Save UI.
  HTTP wiring: `AssetsWeb.VcsController` (x00cd).
  """

  import Ecto.Query, only: [from: 2]

  alias Assets.Blobs
  alias Assets.Blobs.CasObject
  alias Assets.Repo
  alias Assets.Vcs
  alias Assets.Vcs.BlobOrigin
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.CommitParent
  alias Assets.Vcs.Object
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo
  alias Assets.Vcs.RepoBlob
  alias Assets.Vcs.RepoCommit
  alias Assets.VcsObject

  @max_new_commits 256
  @max_object_bytes 1_048_576
  @max_object_hashes 512
  @max_ff_visits 10_000

  @spec negotiate(String.t(), String.t(), map()) ::
          {:ok, %{missing_object_hashes: [String.t()], missing_blob_blake3s: [String.t()]}}
          | {:error, term()}
  def negotiate(repo_id, owner_sub, attrs)
      when is_binary(repo_id) and is_binary(owner_sub) and is_map(attrs) do
    with {:ok, _repo} <- fetch_owned_repo(repo_id, owner_sub),
         {:ok, object_hashes} <- fetch_hash_list(attrs, :object_hashes, @max_object_hashes),
         {:ok, blob_blake3s} <- fetch_hash_list(attrs, :blob_blake3s, @max_object_hashes),
         :ok <- require_keys(attrs, [:ref_name, :new_commit_hash]) do
      {:ok,
       %{
         missing_object_hashes: missing_hashes(object_hashes, &object_exists?/1),
         missing_blob_blake3s: missing_hashes(blob_blake3s, &cas_exists?/1)
       }}
    end
  end

  @spec push(String.t(), String.t(), map()) ::
          {:ok, VcsRef.t()} | {:error, term()}
  def push(repo_id, owner_sub, attrs)
      when is_binary(repo_id) and is_binary(owner_sub) and is_map(attrs) do
    with {:ok, repo} <- fetch_owned_repo(repo_id, owner_sub),
         {:ok, prep} <- prepare_push(repo, owner_sub, attrs) do
      run_push_tx(repo, owner_sub, prep)
    end
  end

  defp run_push_tx(repo, owner_sub, prep) do
    case Repo.transaction(fn -> write_push!(repo, owner_sub, prep) end) do
      {:ok, vref} -> {:ok, vref}
      {:error, reason} -> {:error, reason}
    end
  end

  defp missing_hashes(hashes, exists_fun) do
    hashes
    |> Enum.map(&String.downcase/1)
    |> Enum.uniq()
    |> Enum.reject(exists_fun)
  end

  # ---------------------------------------------------------------------------
  # Prepare (TX outside)
  # ---------------------------------------------------------------------------

  defp prepare_push(%VcsRepo{} = repo, owner_sub, attrs) do
    with {:ok, ref_name} <- fetch_binary(attrs, :ref_name),
         {:ok, new_hash} <- fetch_commit_hash(attrs, :new_commit_hash),
         old_hash <- Map.get(attrs, :old_commit_hash),
         :ok <- validate_old_hash(old_hash),
         old_norm = normalize_old(old_hash),
         :ok <- assert_ref_cas_precheck(repo.repo_id, ref_name, old_norm),
         {:ok, objects} <- fetch_objects(attrs),
         {:ok, verified} <- verify_objects(objects),
         {:ok, trees, push_commits} <- split_verified(verified),
         {:ok, lookup} <- build_commit_lookup(new_hash, push_commits),
         :ok <- assert_tip_in_lookup(new_hash, lookup),
         {:ok, reachable} <- reachable_from_tip(new_hash, lookup),
         :ok <- assert_no_unreachable_push_commits(push_commits, reachable),
         {:ok, joining} <- joining_commits(repo.repo_id, reachable),
         :ok <- assert_joining_limit(joining),
         {:ok, new_commits} <- select_new_commits(push_commits, reachable),
         :ok <- assert_authors(new_commits, owner_sub),
         :ok <- assert_parents_and_generation(new_commits, lookup),
         :ok <- assert_trees_present(joining, trees, lookup),
         {:ok, blob_children} <- collect_blob_children(joining, trees, lookup),
         :ok <- Blobs.assert_child_grants(owner_sub, blob_children),
         :ok <- assert_cas_blobs(blob_children),
         :ok <- assert_fast_forward_push(old_norm, new_hash, lookup),
         {:ok, authorships} <- prepare_authorships(owner_sub, new_commits),
         {:ok, accepted} <- accept_reachable_objects(verified, reachable, lookup) do
      {:ok,
       %{
         ref_name: ref_name,
         old_commit_hash: old_norm,
         new_commit_hash: new_hash,
         trees: trees,
         lookup: lookup,
         joining: joining,
         new_commits: new_commits,
         blob_children: blob_children,
         authorships: authorships,
         verified: accepted
       }}
    end
  end

  defp assert_ref_cas_precheck(repo_id, ref_name, old_hash) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: ref_name) do
      nil when is_nil(old_hash) -> :ok
      nil -> {:error, :cas_mismatch}
      %VcsRef{is_frozen: true} -> {:error, :frozen}
      %VcsRef{protected: true} -> {:error, :ref_protected}
      %VcsRef{commit_hash: ^old_hash} -> :ok
      %VcsRef{} -> {:error, :cas_mismatch}
    end
  end

  defp validate_old_hash(nil), do: :ok
  defp validate_old_hash(hash) when is_binary(hash) and byte_size(hash) == 64, do: :ok
  defp validate_old_hash(_), do: {:error, :invalid_old_commit_hash}

  defp normalize_old(nil), do: nil
  defp normalize_old(hash) when is_binary(hash), do: String.downcase(hash)

  defp fetch_objects(attrs) do
    case Map.get(attrs, :objects) do
      list when is_list(list) and length(list) <= @max_object_hashes ->
        normalize_objects(list)

      list when is_list(list) ->
        {:error, :too_many_objects}

      _ ->
        {:error, :invalid_objects}
    end
  end

  defp normalize_objects(list) do
    list
    |> Enum.reduce_while([], fn item, acc ->
      case normalize_object(item) do
        {:ok, obj} -> {:cont, [obj | acc]}
        {:error, _} = err -> {:halt, err}
      end
    end)
    |> reverse_ok_list()
  end

  defp reverse_ok_list(list) when is_list(list), do: {:ok, Enum.reverse(list)}
  defp reverse_ok_list({:error, _} = err), do: err

  defp normalize_object(%{hash_hex: hash, bytes: bytes})
       when is_binary(hash) and is_binary(bytes) and byte_size(bytes) <= @max_object_bytes and
              byte_size(hash) == 64 do
    {:ok, %{hash_hex: String.downcase(hash), bytes: bytes}}
  end

  defp normalize_object(%{hash_hex: _hash, bytes: bytes})
       when is_binary(bytes) and byte_size(bytes) > @max_object_bytes do
    {:error, :object_too_large}
  end

  defp normalize_object(%{"hash_hex" => hash, "bytes" => bytes}),
    do: normalize_object(%{hash_hex: hash, bytes: bytes})

  defp normalize_object(_), do: {:error, :invalid_objects}

  defp verify_objects(objects) do
    objects
    |> Enum.reduce_while([], fn obj, acc ->
      case verify_one_object(obj) do
        {:ok, verified} -> {:cont, [verified | acc]}
        {:error, _} = err -> {:halt, err}
      end
    end)
    |> reverse_ok_list()
  end

  defp verify_one_object(%{hash_hex: hash, bytes: bytes}) do
    case VcsObject.verify(bytes) do
      {:ok, %{"hash_hex" => ^hash} = summary} ->
        {:ok, %{hash_hex: hash, bytes: bytes, summary: summary}}

      {:ok, %{"hash_hex" => other}} ->
        {:error, {:hash_mismatch, hash, other}}

      {:error, reason} ->
        {:error, {:verify_failed, hash, reason}}
    end
  end

  defp split_verified(verified) do
    trees =
      verified
      |> Enum.filter(&(&1.summary["kind"] == "tree"))
      |> Map.new(&{&1.hash_hex, &1})

    commits =
      verified
      |> Enum.filter(&(&1.summary["kind"] == "commit"))
      |> Map.new(&{&1.hash_hex, &1})

    if map_size(trees) + map_size(commits) == length(verified) do
      {:ok, trees, commits}
    else
      {:error, :invalid_object_kind}
    end
  end

  defp build_commit_lookup(tip_hash, push_commits) do
    seed =
      [tip_hash | Enum.flat_map(Map.values(push_commits), &List.wrap(&1.summary["parents_hex"]))]
      |> Enum.uniq()

    expand_commit_lookup(MapSet.new(seed), push_commits, %{}, 0)
  end

  defp expand_commit_lookup(_needed, _push, _acc, visits) when visits > @max_ff_visits do
    {:error, :commit_graph_too_deep}
  end

  defp expand_commit_lookup(needed, push_commits, acc, visits) do
    from_push = merge_push_entries(needed, push_commits, acc)

    missing =
      needed
      |> Enum.reject(&Map.has_key?(from_push, &1))

    db_rows = load_commits_and_parents(missing)
    merged = Map.merge(from_push, db_rows)

    still_missing = Enum.reject(needed, &Map.has_key?(merged, &1))

    if still_missing != [] do
      {:error, {:missing_commit, hd(still_missing)}}
    else
      parent_hashes =
        merged
        |> Map.values()
        |> Enum.flat_map(& &1.parents)
        |> Enum.reject(&Map.has_key?(merged, &1))

      if parent_hashes == [] do
        {:ok, merged}
      else
        next_needed = MapSet.new(Map.keys(merged) ++ parent_hashes)
        expand_commit_lookup(next_needed, push_commits, merged, visits + 1)
      end
    end
  end

  defp merge_push_entries(needed, push_commits, acc) do
    Enum.reduce(needed, acc, fn hash, map ->
      case Map.get(push_commits, hash) do
        %{summary: s} = push ->
          Map.put(map, hash, %{
            generation: s["generation"],
            parents: List.wrap(s["parents_hex"]),
            tree_hash: s["tree_hex"],
            author_sub: s["author_sub"],
            source: :push,
            push: push
          })

        nil ->
          map
      end
    end)
  end

  defp load_commits_and_parents([]), do: %{}

  defp load_commits_and_parents(hashes) do
    commits =
      from(c in Commit, where: c.commit_hash in ^hashes)
      |> Repo.all()
      |> Map.new(&{&1.commit_hash, &1})

    parents_by =
      from(p in CommitParent,
        where: p.commit_hash in ^hashes,
        order_by: [asc: p.idx]
      )
      |> Repo.all()
      |> Enum.group_by(& &1.commit_hash, & &1.parent_hash)

    hashes
    |> Enum.reduce(%{}, fn hash, acc ->
      case Map.get(commits, hash) do
        %Commit{} = c ->
          Map.put(acc, hash, %{
            generation: c.generation,
            parents: Map.get(parents_by, hash, []),
            tree_hash: c.tree_hash,
            author_sub: c.author_sub,
            source: :db
          })

        nil ->
          acc
      end
    end)
  end

  defp assert_tip_in_lookup(tip, lookup) do
    if Map.has_key?(lookup, tip), do: :ok, else: {:error, :missing_tip_commit}
  end

  defp reachable_from_tip(tip, lookup), do: walk_reachable([tip], MapSet.new(), lookup, 0)

  defp walk_reachable([], seen, _lookup, _visits), do: {:ok, seen}

  defp walk_reachable(_queue, _seen, _lookup, visits) when visits > @max_ff_visits do
    {:error, :commit_graph_too_deep}
  end

  defp walk_reachable([hash | rest], seen, lookup, visits) do
    if MapSet.member?(seen, hash) do
      walk_reachable(rest, seen, lookup, visits)
    else
      case Map.fetch(lookup, hash) do
        {:ok, %{parents: parents}} ->
          walk_reachable(parents ++ rest, MapSet.put(seen, hash), lookup, visits + 1)

        :error ->
          {:error, {:missing_commit, hash}}
      end
    end
  end

  defp assert_no_unreachable_push_commits(push_commits, reachable) do
    unreachable =
      push_commits
      |> Map.keys()
      |> Enum.reject(&MapSet.member?(reachable, &1))

    if unreachable == [], do: :ok, else: {:error, {:unreachable_commit, hd(unreachable)}}
  end

  defp joining_commits(repo_id, reachable) do
    hashes = MapSet.to_list(reachable)

    present =
      from(rc in RepoCommit,
        where: rc.repo_id == ^repo_id and rc.commit_hash in ^hashes,
        select: rc.commit_hash
      )
      |> Repo.all()
      |> MapSet.new()

    {:ok, MapSet.difference(reachable, present)}
  end

  defp select_new_commits(push_commits, reachable) do
    push_hashes =
      push_commits
      |> Map.keys()
      |> Enum.filter(&MapSet.member?(reachable, &1))

    existing =
      push_hashes
      |> load_commits_and_parents()
      |> Map.keys()
      |> MapSet.new()

    new =
      push_hashes
      |> Enum.reject(&MapSet.member?(existing, &1))
      |> Enum.map(&Map.fetch!(push_commits, &1))

    {:ok, new}
  end

  defp assert_joining_limit(joining) do
    if MapSet.size(joining) <= @max_new_commits,
      do: :ok,
      else: {:error, :too_many_commits}
  end

  # Only tip-reachable commits and their trees are stored (orphan trees dropped).
  defp accept_reachable_objects(verified, reachable, lookup) do
    needed_trees =
      reachable
      |> Enum.map(&get_in(lookup, [&1, :tree_hash]))
      |> Enum.filter(&is_binary/1)
      |> MapSet.new()

    accepted =
      Enum.filter(verified, fn %{hash_hex: hash, summary: summary} ->
        case summary["kind"] do
          "commit" -> MapSet.member?(reachable, hash)
          "tree" -> MapSet.member?(needed_trees, hash)
          _ -> false
        end
      end)

    {:ok, accepted}
  end

  defp assert_authors(new_commits, owner_sub) do
    Enum.reduce_while(new_commits, :ok, fn %{summary: summary}, :ok ->
      if summary["author_sub"] == owner_sub,
        do: {:cont, :ok},
        else: {:halt, {:error, :author_mismatch}}
    end)
  end

  defp assert_parents_and_generation(new_commits, lookup) do
    Enum.reduce_while(new_commits, :ok, fn commit, :ok ->
      case assert_one_commit_parents(commit, lookup) do
        :ok -> {:cont, :ok}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp assert_one_commit_parents(%{summary: summary}, lookup) do
    parents = List.wrap(summary["parents_hex"])

    with :ok <- assert_parents_present(parents, lookup) do
      assert_generation(parents, summary["generation"], lookup)
    end
  end

  defp assert_parents_present(parents, lookup) do
    Enum.reduce_while(parents, :ok, fn parent, :ok ->
      if Map.has_key?(lookup, parent),
        do: {:cont, :ok},
        else: {:halt, {:error, {:missing_parent, parent}}}
    end)
  end

  defp assert_generation([], 1, _lookup), do: :ok
  defp assert_generation([], _generation, _lookup), do: {:error, :invalid_generation}

  defp assert_generation(parents, generation, lookup) when is_integer(generation) do
    gens = Enum.map(parents, &lookup_generation(&1, lookup))

    cond do
      Enum.any?(gens, &is_nil/1) -> {:error, :missing_parent}
      generation == Enum.max(gens) + 1 -> :ok
      true -> {:error, :invalid_generation}
    end
  end

  defp assert_generation(_, _, _), do: {:error, :invalid_generation}

  defp lookup_generation(hash, lookup) do
    case Map.get(lookup, hash) do
      %{generation: g} when is_integer(g) -> g
      _ -> nil
    end
  end

  defp assert_trees_present(joining, trees, lookup) do
    Enum.reduce_while(MapSet.to_list(joining), :ok, fn hash, :ok ->
      tree_hex = get_in(lookup, [hash, :tree_hash])

      case tree_present?(tree_hex, trees) do
        :ok -> {:cont, :ok}
        {:error, _} = err -> {:halt, err}
      end
    end)
  end

  defp tree_present?(tree_hex, _trees) when not is_binary(tree_hex), do: {:error, :missing_tree}

  defp tree_present?(tree_hex, trees) do
    cond do
      Map.has_key?(trees, tree_hex) -> :ok
      match?({:ok, %Object{kind: "tree"}}, Vcs.get_object(tree_hex)) -> :ok
      true -> {:error, {:missing_tree, tree_hex}}
    end
  end

  defp collect_blob_children(joining, trees, lookup) do
    joining
    |> MapSet.to_list()
    |> Enum.reduce_while({:ok, []}, fn hash, {:ok, acc} ->
      tree_hex = get_in(lookup, [hash, :tree_hash])

      case tree_children(tree_hex, trees) do
        {:ok, children} -> {:cont, {:ok, children ++ acc}}
        {:error, _} = err -> {:halt, err}
      end
    end)
    |> case do
      {:ok, list} -> {:ok, Enum.uniq_by(list, & &1.blake3)}
      err -> err
    end
  end

  defp tree_children(tree_hex, _trees) when not is_binary(tree_hex), do: {:error, :missing_tree}

  defp tree_children(tree_hex, trees) do
    case Map.get(trees, tree_hex) do
      %{summary: %{"child_blake3s_hex" => children}} when is_list(children) ->
        {:ok, Enum.map(children, &%{blake3: &1})}

      %{summary: _} ->
        {:error, {:corrupt_tree, tree_hex}}

      nil ->
        tree_children_from_db(tree_hex)
    end
  end

  defp tree_children_from_db(tree_hex) do
    case Vcs.get_object(tree_hex) do
      {:ok, %Object{kind: "tree", bytes: bytes}} ->
        verify_tree_children(tree_hex, bytes)

      {:ok, %Object{kind: kind}} ->
        {:error, {:wrong_tree_kind, tree_hex, kind}}

      {:error, reason} ->
        {:error, {:missing_tree, tree_hex, reason}}
    end
  end

  defp verify_tree_children(tree_hex, bytes) do
    case VcsObject.verify(bytes) do
      {:ok, %{"kind" => "tree", "hash_hex" => ^tree_hex, "child_blake3s_hex" => children}}
      when is_list(children) ->
        {:ok, Enum.map(children, &%{blake3: &1})}

      {:ok, summary} ->
        {:error, {:corrupt_tree, tree_hex, summary}}

      {:error, reason} ->
        {:error, {:tree_verify_failed, tree_hex, reason}}
    end
  end

  defp assert_cas_blobs(children) do
    Enum.reduce_while(children, :ok, fn %{blake3: blake3}, :ok ->
      if cas_exists?(blake3),
        do: {:cont, :ok},
        else: {:halt, {:error, {:missing_blob, blake3}}}
    end)
  end

  defp assert_fast_forward_push(nil, _new_hash, _lookup), do: :ok

  defp assert_fast_forward_push(old_hash, new_hash, lookup) do
    case Map.fetch(lookup, old_hash) do
      {:ok, %{generation: old_gen}} ->
        do_ff_check(old_hash, new_hash, old_gen, @max_ff_visits, lookup)

      :error ->
        {:error, :not_fast_forward}
    end
  end

  defp do_ff_check(old, current, _old_gen, _visits, _lookup) when old == current, do: :ok
  defp do_ff_check(_old, _current, _old_gen, 0, _lookup), do: {:error, :not_fast_forward}

  defp do_ff_check(old, current, old_gen, visits, lookup) do
    with {:ok, meta} <- Map.fetch(lookup, current),
         :ok <- assert_ff_generation(meta.generation, old_gen),
         {:ok, parent} <- first_parent(meta.parents) do
      do_ff_check(old, parent, old_gen, visits - 1, lookup)
    else
      :error -> {:error, :not_fast_forward}
      {:error, _} = err -> err
    end
  end

  defp assert_ff_generation(gen, old_gen) when gen < old_gen, do: {:error, :not_fast_forward}
  defp assert_ff_generation(_gen, _old_gen), do: :ok

  defp first_parent([parent | _]), do: {:ok, parent}
  defp first_parent(_), do: {:error, :not_fast_forward}

  defp prepare_authorships(owner_sub, new_commits) do
    new_commits
    |> Enum.reduce_while([], fn %{hash_hex: hash}, acc ->
      case Cert.prepare_authorship_unless_present(owner_sub, hash) do
        {:ok, prepared} -> {:cont, [{hash, prepared} | acc]}
        {:error, _} = err -> {:halt, err}
      end
    end)
    |> reverse_ok_list()
  end

  # ---------------------------------------------------------------------------
  # Write (TX inside)
  # ---------------------------------------------------------------------------

  defp write_push!(%VcsRepo{} = repo, owner_sub, prep) do
    case fetch_owned_repo(repo.repo_id, owner_sub) do
      {:ok, _} -> :ok
      {:error, reason} -> Repo.rollback(reason)
    end

    # Fail-closed: re-check Grant + CAS inside the TX (promote と同型).
    assert_child_grants!(owner_sub, prep.blob_children)
    assert_cas_blobs!(prep.blob_children)

    Enum.each(prep.verified, fn %{hash_hex: hash, bytes: bytes, summary: summary} ->
      insert_object!(hash, summary["kind"], bytes)
    end)

    Enum.each(prep.new_commits, fn %{hash_hex: hash, summary: summary} ->
      insert_commit_row!(hash, summary)
      insert_parents!(hash, List.wrap(summary["parents_hex"]))
    end)

    Enum.each(MapSet.to_list(prep.joining), fn hash ->
      insert_repo_commit!(repo.repo_id, hash)
    end)

    insert_repo_blobs!(repo.repo_id, prep.blob_children)

    # Oldest-first so first-seen origin prefers the earlier joining commit.
    prep.joining
    |> joining_sorted(prep.lookup)
    |> Enum.each(fn hash -> insert_origins_for_commit!(hash, prep) end)

    Enum.each(prep.authorships, fn {_hash, prepared} ->
      Cert.insert_prepared_authorship!(prepared)
    end)

    update_or_create_ref!(
      repo.repo_id,
      prep.ref_name,
      prep.old_commit_hash,
      prep.new_commit_hash
    )
  end

  defp assert_child_grants!(owner_sub, children) do
    case Blobs.assert_child_grants(owner_sub, children) do
      :ok -> :ok
      {:error, reason} -> Repo.rollback(reason)
    end
  end

  defp assert_cas_blobs!(children) do
    case assert_cas_blobs(children) do
      :ok -> :ok
      {:error, reason} -> Repo.rollback(reason)
    end
  end

  defp insert_origins_for_commit!(hash, prep) do
    tree_hex = get_in(prep.lookup, [hash, :tree_hash])

    case get_in(prep.lookup, [hash, :author_sub]) do
      author_sub when is_binary(author_sub) ->
        insert_origins_with_author!(hash, tree_hex, prep.trees, author_sub)

      _ ->
        Repo.rollback({:missing_commit_author, hash})
    end
  end

  defp insert_origins_with_author!(hash, tree_hex, trees, author_sub) do
    case tree_children(tree_hex, trees) do
      {:ok, children} -> insert_blob_origins!(children, hash, author_sub)
      {:error, reason} -> Repo.rollback(reason)
    end
  end

  # Stable: generation asc, then commit_hash asc (same-gen first_commit_hash deterministic).
  defp joining_sorted(joining, lookup) do
    joining
    |> MapSet.to_list()
    |> Enum.sort_by(fn hash -> {get_in(lookup, [hash, :generation]) || 0, hash} end)
  end

  defp update_or_create_ref!(repo_id, ref_name, old_hash, new_hash) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: ref_name) do
      nil -> create_or_reject_missing_ref!(repo_id, ref_name, old_hash, new_hash)
      %VcsRef{} = vref -> advance_existing_ref!(vref, old_hash, new_hash)
    end
  end

  defp create_or_reject_missing_ref!(repo_id, ref_name, nil, new_hash) do
    create_branch_ref!(repo_id, ref_name, new_hash)
  end

  defp create_or_reject_missing_ref!(_repo_id, _ref_name, _old_hash, _new_hash) do
    Repo.rollback(:cas_mismatch)
  end

  defp advance_existing_ref!(%VcsRef{is_frozen: true}, _old, _new), do: Repo.rollback(:frozen)
  defp advance_existing_ref!(%VcsRef{kind: "tag"}, _old, _new), do: Repo.rollback(:frozen)

  defp advance_existing_ref!(%VcsRef{protected: true}, _old, _new),
    do: Repo.rollback(:ref_protected)

  defp advance_existing_ref!(%VcsRef{commit_hash: current} = vref, old_hash, new_hash) do
    if current == old_hash do
      apply_ref_advance!(vref, new_hash)
    else
      Repo.rollback(:cas_mismatch)
    end
  end

  defp apply_ref_advance!(vref, new_hash) do
    case Repo.update(VcsRef.advance_changeset(vref, %{commit_hash: new_hash})) do
      {:ok, updated} -> updated
      {:error, cs} -> Repo.rollback({:ref_update_failed, cs})
    end
  end

  defp create_branch_ref!(repo_id, name, commit_hash) do
    unless Vcs.commit_in_repo?(repo_id, commit_hash), do: Repo.rollback(:not_found)

    attrs = %{
      ref_id: Vcs.generate_id(),
      repo_id: repo_id,
      name: name,
      kind: "branch",
      commit_hash: commit_hash,
      is_frozen: false,
      protected: false
    }

    case Repo.insert(VcsRef.changeset(%VcsRef{}, attrs)) do
      {:ok, vref} -> vref
      {:error, cs} -> Repo.rollback({:ref_create_failed, cs})
    end
  end

  defp insert_object!(hash, kind, bytes) do
    case Repo.get(Object, hash) do
      %Object{kind: ^kind, bytes: ^bytes} -> :ok
      %Object{} -> Repo.rollback({:object_content_mismatch, hash})
      nil -> insert_object_new!(hash, kind, bytes)
    end
  end

  defp insert_object_new!(hash, kind, bytes) do
    case Repo.insert(
           Object.changeset(%Object{}, %{hash: hash, kind: kind, bytes: bytes}),
           on_conflict: :nothing,
           conflict_target: :hash
         ) do
      {:ok, _} -> assert_object_bytes!(hash, kind, bytes)
      {:error, cs} -> Repo.rollback({:object_insert_failed, cs})
    end
  end

  defp assert_object_bytes!(hash, kind, bytes) do
    case Repo.get(Object, hash) do
      %Object{kind: ^kind, bytes: ^bytes} -> :ok
      %Object{} -> Repo.rollback({:object_content_mismatch, hash})
      nil -> Repo.rollback({:object_insert_failed, :missing_after_insert})
    end
  end

  defp insert_commit_row!(hash, summary) do
    attrs = commit_attrs(hash, summary)

    case Repo.get(Commit, hash) do
      %Commit{} = existing -> assert_commit_attrs!(existing, attrs)
      nil -> insert_commit_new!(attrs)
    end
  end

  defp commit_attrs(hash, summary) do
    %{
      commit_hash: hash,
      tree_hash: summary["tree_hex"],
      generation: summary["generation"],
      author_sub: summary["author_sub"],
      created_at_ms: summary["created_at_ms"],
      message: summary["message"] || ""
    }
  end

  defp insert_commit_new!(attrs) do
    case Repo.insert(Commit.changeset(%Commit{}, attrs),
           on_conflict: :nothing,
           conflict_target: :commit_hash
         ) do
      {:ok, _} -> assert_commit_after_insert!(attrs)
      {:error, cs} -> Repo.rollback({:commit_insert_failed, cs})
    end
  end

  defp assert_commit_after_insert!(attrs) do
    case Repo.get(Commit, attrs.commit_hash) do
      %Commit{} = c -> assert_commit_attrs!(c, attrs)
      nil -> Repo.rollback({:commit_insert_failed, :missing_after_insert})
    end
  end

  defp assert_commit_attrs!(%Commit{} = c, attrs) do
    if commit_match?(c, attrs),
      do: :ok,
      else: Repo.rollback({:commit_content_mismatch, c.commit_hash})
  end

  defp commit_match?(%Commit{} = c, attrs) do
    c.tree_hash == attrs.tree_hash and
      c.generation == attrs.generation and
      c.author_sub == attrs.author_sub and
      c.created_at_ms == attrs.created_at_ms and
      c.message == attrs.message
  end

  defp insert_parents!(commit_hash, parents) do
    parents
    |> Enum.with_index()
    |> Enum.each(fn {parent_hash, idx} -> insert_one_parent!(commit_hash, idx, parent_hash) end)
  end

  defp insert_one_parent!(commit_hash, idx, parent_hash) do
    case Repo.get_by(CommitParent, commit_hash: commit_hash, idx: idx) do
      %CommitParent{parent_hash: ^parent_hash} -> :ok
      %CommitParent{} -> Repo.rollback({:parent_mismatch, commit_hash, idx})
      nil -> insert_parent_row!(commit_hash, idx, parent_hash)
    end
  end

  defp insert_parent_row!(commit_hash, idx, parent_hash) do
    case Repo.insert(
           CommitParent.changeset(%CommitParent{}, %{
             commit_hash: commit_hash,
             idx: idx,
             parent_hash: parent_hash
           }),
           on_conflict: :nothing,
           conflict_target: [:commit_hash, :idx]
         ) do
      {:ok, _} -> assert_parent_row!(commit_hash, idx, parent_hash)
      {:error, cs} -> Repo.rollback({:parent_insert_failed, cs})
    end
  end

  defp assert_parent_row!(commit_hash, idx, parent_hash) do
    case Repo.get_by(CommitParent, commit_hash: commit_hash, idx: idx) do
      %CommitParent{parent_hash: ^parent_hash} -> :ok
      %CommitParent{} -> Repo.rollback({:parent_mismatch, commit_hash, idx})
      nil -> Repo.rollback({:parent_insert_failed, :missing_after_insert})
    end
  end

  defp insert_repo_commit!(repo_id, commit_hash) do
    case Repo.insert(
           RepoCommit.changeset(%RepoCommit{}, %{repo_id: repo_id, commit_hash: commit_hash}),
           on_conflict: :nothing,
           conflict_target: [:repo_id, :commit_hash]
         ) do
      {:ok, _} -> :ok
      {:error, cs} -> Repo.rollback({:repo_commit_insert_failed, cs})
    end
  end

  defp insert_repo_blobs!(repo_id, children) when is_list(children) do
    children
    |> Enum.map(fn %{blake3: blake3} -> String.downcase(blake3) end)
    |> Enum.uniq()
    |> Enum.each(fn blake3 -> insert_one_repo_blob!(repo_id, blake3) end)
  end

  defp insert_one_repo_blob!(repo_id, blake3) do
    case Repo.insert(
           RepoBlob.changeset(%RepoBlob{}, %{repo_id: repo_id, blake3: blake3}),
           on_conflict: :nothing,
           conflict_target: [:repo_id, :blake3]
         ) do
      {:ok, _} -> :ok
      {:error, cs} -> Repo.rollback({:repo_blob_insert_failed, cs})
    end
  end

  defp insert_blob_origins!(children, commit_hash, author_sub) when is_list(children) do
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    children
    |> Enum.map(fn %{blake3: blake3} -> String.downcase(blake3) end)
    |> Enum.uniq()
    |> Enum.each(fn blake3 -> insert_one_origin!(blake3, commit_hash, author_sub, now) end)
  end

  defp insert_one_origin!(blake3, commit_hash, author_sub, now) do
    case Repo.insert(
           BlobOrigin.changeset(%BlobOrigin{}, %{
             blake3: blake3,
             first_commit_hash: commit_hash,
             author_sub: author_sub,
             first_seen_at: now
           }),
           on_conflict: :nothing,
           conflict_target: :blake3
         ) do
      {:ok, _} -> :ok
      {:error, cs} -> Repo.rollback({:blob_origin_insert_failed, cs})
    end
  end

  defp fetch_owned_repo(repo_id, owner_sub) do
    case Repo.get(VcsRepo, repo_id) do
      nil -> {:error, :not_found}
      %VcsRepo{owner_sub: ^owner_sub} = repo -> {:ok, repo}
      %VcsRepo{} -> {:error, :forbidden}
    end
  end

  defp object_exists?(hash), do: Repo.exists?(from(o in Object, where: o.hash == ^hash))
  defp cas_exists?(blake3), do: Repo.exists?(from(c in CasObject, where: c.blake3 == ^blake3))

  defp require_keys(attrs, keys) do
    if Enum.all?(keys, &Map.has_key?(attrs, &1)), do: :ok, else: {:error, :invalid_attrs}
  end

  defp fetch_binary(attrs, key) do
    case Map.get(attrs, key) do
      v when is_binary(v) and byte_size(v) > 0 -> {:ok, v}
      _ -> {:error, {:invalid_attr, key}}
    end
  end

  defp fetch_commit_hash(attrs, key) do
    case Map.get(attrs, key) do
      v when is_binary(v) and byte_size(v) == 64 -> {:ok, String.downcase(v)}
      _ -> {:error, {:invalid_attr, key}}
    end
  end

  defp fetch_hash_list(attrs, key, max) do
    case Map.get(attrs, key) do
      list when is_list(list) and length(list) <= max ->
        validate_hash_list(list, key)

      list when is_list(list) ->
        {:error, :too_many_hashes}

      _ ->
        {:error, {:invalid_attr, key}}
    end
  end

  defp validate_hash_list(list, key) do
    if Enum.all?(list, &(is_binary(&1) and byte_size(&1) == 64)) do
      {:ok, list}
    else
      {:error, {:invalid_attr, key}}
    end
  end
end
