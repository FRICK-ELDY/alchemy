defmodule World.MtlsIssuer do
  @moduledoc """
  x0066: Sign short-lived mTLS client certs from a CSR using openssl CLI (no Hex x509 dep).

  Guarantees: CSR Subject is ignored; CN is forced from server-authorized role
  (`alchemy-host` | `alchemy-guest`); CA key / CSR scratch files are removed even on
  sign failure; **openssl NotAfter ≈ soft TTL** (`expires_at`, default 300s) via
  `-not_after` (x0068). Non-guarantees: CRL/OCSP; sub-second clock skew.
  """

  require Logger

  @cn_host "alchemy-host"
  @cn_guest "alchemy-guest"
  # Soft refresh SLA + certificate NotAfter (aligned with room_token_ttl_seconds).
  @default_ttl_seconds 300

  @type role :: String.t()

  @doc """
  True when CA cert + key PEM (or readable path files) are configured.
  """
  def configured? do
    match?({:ok, _}, resolve_ca())
  end

  @doc """
  Sign `csr_pem` for an authorized `role` (`"host"` | `"guest"`).

  Returns PEM cert + CA PEM + `expires_at` (UTC soft TTL). Does not return the
  client private key (client already holds it).
  """
  @spec issue(String.t(), role(), keyword()) ::
          {:ok, %{client_cert_pem: String.t(), ca_cert_pem: String.t(), expires_at: DateTime.t()}}
          | {:error, atom()}
  def issue(csr_pem, role, opts \\ [])

  def issue(csr_pem, role, opts)
      when is_binary(csr_pem) and csr_pem != "" and role in ["host", "guest"] do
    ttl = Keyword.get(opts, :ttl_seconds, @default_ttl_seconds)
    cn = if(role == "host", do: @cn_host, else: @cn_guest)
    work = work_dir()

    with :ok <- File.mkdir_p(work),
         {:ok, ca_spec} <- resolve_ca(),
         {:ok, ca_cert_path, ca_key_path, ca_pem, temp_ca_files} <- materialize_ca(work, ca_spec) do
      try do
        with {:ok, paths} <- write_inputs(work, csr_pem),
             :ok <- run_sign(paths, ca_cert_path, ca_key_path, cn, ttl),
             {:ok, cert_pem} <- File.read(paths.cert) do
          now = DateTime.utc_now() |> DateTime.truncate(:second)
          expires_at = DateTime.add(now, ttl, :second)

          {:ok,
           %{
             client_cert_pem: cert_pem,
             ca_cert_pem: ca_pem,
             expires_at: expires_at
           }}
        else
          {:error, reason} = err when is_atom(reason) ->
            err

          {:error, reason} ->
            Logger.warning("mtls_issuer failed: #{inspect(reason)}")
            {:error, :sign_failed}
        end
      after
        # CSR / issued cert / ext + any temp CA copies (inline PEM path).
        maybe_cleanup_inputs()
        Enum.each(temp_ca_files, &File.rm/1)
      end
    end
  end

  def issue(_, _, _), do: {:error, :invalid_args}

  defp work_dir do
    Application.get_env(:world, :mtls_work_dir) ||
      Path.join(System.tmp_dir!(), "alchemy-world-mtls")
  end

  defp openssl_bin do
    System.get_env("OPENSSL_BIN") ||
      Application.get_env(:world, :openssl_bin) ||
      detect_openssl()
  end

  defp detect_openssl do
    [
      "openssl",
      "C:/Program Files/Git/usr/bin/openssl.exe",
      "C:/Program Files/OpenSSL-Win64/bin/openssl.exe"
    ]
    |> Enum.find(&openssl_ok?/1) || "openssl"
  end

  defp openssl_ok?(bin) do
    match?({_, 0}, System.cmd(bin, ["version"], stderr_to_stdout: true))
  rescue
    _ -> false
  end

  # {:inline, cert_pem, key_pem} | {:paths, cert_path, key_path}
  defp resolve_ca do
    case resolve_ca_inline() do
      {:ok, _} = ok -> ok
      :none -> resolve_ca_paths()
    end
  end

  defp resolve_ca_inline do
    cert = Application.get_env(:world, :mtls_ca_cert_pem)
    key = Application.get_env(:world, :mtls_ca_key_pem)

    if present_bin?(cert) and present_bin?(key) do
      {:ok, {:inline, cert, key}}
    else
      :none
    end
  end

  defp resolve_ca_paths do
    cert_path = Application.get_env(:world, :mtls_ca_cert_path)
    key_path = Application.get_env(:world, :mtls_ca_key_path)

    cond do
      not (present_bin?(cert_path) and present_bin?(key_path)) ->
        {:error, :ca_not_configured}

      File.regular?(cert_path) and File.regular?(key_path) ->
        {:ok, {:paths, cert_path, key_path}}

      true ->
        {:error, :ca_unreadable}
    end
  end

  defp present_bin?(v), do: is_binary(v) and v != ""

  # Path-configured CA: openssl uses files in place (no CA key copy into work dir).
  # Inline PEM: write temp copies with 0600; always deleted in `after`.
  defp materialize_ca(_work, {:paths, cert_path, key_path}) do
    case File.read(cert_path) do
      {:ok, ca_pem} -> {:ok, cert_path, key_path, ca_pem, []}
      {:error, _} -> {:error, :ca_unreadable}
    end
  end

  defp materialize_ca(work, {:inline, cert_pem, key_pem}) do
    stamp = System.system_time(:nanosecond)
    cert_path = Path.join(work, "ca-#{stamp}.cert.pem")
    key_path = Path.join(work, "ca-#{stamp}.key.pem")

    with :ok <- File.write(cert_path, cert_pem),
         :ok <- File.write(key_path, key_pem),
         :ok <- chmod_secret(key_path) do
      {:ok, cert_path, key_path, cert_pem, [cert_path, key_path]}
    else
      {:error, :chmod_failed} ->
        File.rm(cert_path)
        File.rm(key_path)
        {:error, :chmod_failed}

      {:error, reason} ->
        File.rm(cert_path)
        File.rm(key_path)
        Logger.warning("mtls_issuer materialize_ca write failed: #{inspect(reason)}")
        {:error, :sign_failed}
    end
  end

  defp write_inputs(work, csr_pem) do
    stamp = System.system_time(:nanosecond)
    csr = Path.join(work, "req-#{stamp}.csr.pem")
    cert = Path.join(work, "req-#{stamp}.cert.pem")
    ext = Path.join(work, "req-#{stamp}.ext")

    ext_body =
      "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n"

    # Register CSR immediately so `after` cleans up if ext write fails.
    Process.put({__MODULE__, :input_paths}, %{csr: csr, cert: cert, ext: ext})

    with :ok <- File.write(csr, csr_pem),
         :ok <- File.write(ext, ext_body) do
      {:ok, %{csr: csr, cert: cert, ext: ext}}
    else
      {:error, reason} ->
        maybe_cleanup_inputs()
        {:error, reason}
    end
  end

  defp maybe_cleanup_inputs do
    case Process.get({__MODULE__, :input_paths}) do
      %{csr: _, cert: _, ext: _} = paths ->
        Process.delete({__MODULE__, :input_paths})
        cleanup_inputs(paths)

      _ ->
        :ok
    end
  end

  defp cleanup_inputs(paths) do
    Enum.each([paths.csr, paths.cert, paths.ext], fn
      p when is_binary(p) -> File.rm(p)
      _ -> :ok
    end)

    :ok
  end

  defp run_sign(paths, ca_cert, ca_key, cn, ttl_seconds) do
    serial = Integer.to_string(System.system_time(:nanosecond), 16)
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    # Skew allowance so zenohd "not yet valid" is unlikely under mild clock drift.
    not_before = DateTime.add(now, -60, :second) |> asn1_utc()
    not_after = DateTime.add(now, ttl_seconds, :second) |> asn1_utc()

    # -subj replaces CSR subject (do not trust client-supplied CN).
    # -not_after overrides -days (OpenSSL 3+); fail-closed if unsupported.
    args = [
      "x509",
      "-req",
      "-in",
      paths.csr,
      "-CA",
      ca_cert,
      "-CAkey",
      ca_key,
      "-set_serial",
      "0x#{serial}",
      "-out",
      paths.cert,
      "-not_before",
      not_before,
      "-not_after",
      not_after,
      "-subj",
      "/CN=#{cn}",
      "-extfile",
      paths.ext
    ]

    case System.cmd(openssl_bin(), args, stderr_to_stdout: true) do
      {_, 0} ->
        :ok

      {out, code} ->
        Logger.warning("openssl x509 failed code=#{code}: #{String.slice(out, 0, 500)}")
        {:error, :openssl_failed}
    end
  end

  defp asn1_utc(%DateTime{} = dt) do
    {{y, m, d}, {hh, mm, ss}} = DateTime.to_naive(dt) |> NaiveDateTime.to_erl()

    :io_lib.format("~4..0B~2..0B~2..0B~2..0B~2..0B~2..0BZ", [y, m, d, hh, mm, ss])
    |> IO.iodata_to_binary()
  end

  defp chmod_secret(path) do
    case File.chmod(path, 0o600) do
      :ok ->
        :ok

      {:error, reason} ->
        Logger.error("mtls_issuer chmod 0600 failed path=#{path} reason=#{inspect(reason)}")
        File.rm(path)
        {:error, :chmod_failed}
    end
  end
end
