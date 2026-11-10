defmodule Assets.CasTmp do
  @moduledoc """
  Per-test CAS path isolation for ExUnit (x0086).

  - Entry point: `use Assets.DataCase, async: false, cas: true` (or ConnCase).
    That injects `@moduletag :tmp_dir`; this module points `:cas_path` at it.
  - `Application.put_env(:assets, :cas_path, …)` is **VM-global**; `cas: true`
    requires `async: false` (enforced at `use` compile time and again here).
  - Suites that omit `cas: true` leave the boot path alone.
  """

  # Mirror of config/test.exs `:cas_path`. **SSoT for Application env is config/test.exs**;
  # this constant is only for assertions (must stay identical or isolation tests fail).
  @boot_cas_path "priv/cas_test"

  @doc "Assertion helper mirroring boot `:cas_path` in `config/test.exs` (not the config SSoT)."
  def boot_cas_path, do: @boot_cas_path

  @doc """
  Compile-time guard for `use …, cas: true, async: true`.
  """
  def reject_async_cas!(opts) when is_list(opts) do
    if Keyword.get(opts, :cas, false) and Keyword.get(opts, :async, false) do
      raise ArgumentError, """
      x0086: `cas: true` requires `async: false` \
      (Application.put_env(:assets, :cas_path) is VM-global).
      """
    end

    :ok
  end

  @doc """
  When ExUnit provides `tmp_dir`, point `:cas_path` at it and restore on exit.

  Returns a context map (`%{cas_path: path}`) or `%{}` when not isolating.
  """
  def install(tags) when is_map(tags) do
    case tags[:tmp_dir] do
      path when is_binary(path) ->
        # Belt-and-suspenders for manual `@moduletag :tmp_dir` without `cas: true`.
        if tags[:async] do
          raise """
          x0086: CAS tmp_dir isolation requires async: false \
          (Application.put_env(:assets, :cas_path) is VM-global).
          """
        end

        prev = Application.get_env(:assets, :cas_path)
        Application.put_env(:assets, :cas_path, path)
        File.mkdir_p!(path)

        ExUnit.Callbacks.on_exit(fn ->
          Application.put_env(:assets, :cas_path, prev)
        end)

        %{cas_path: path}

      _ ->
        %{}
    end
  end
end
