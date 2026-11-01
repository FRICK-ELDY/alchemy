defmodule World.MtlsIssuerTest do
  use ExUnit.Case, async: false

  alias World.MtlsIssuer

  @moduletag :tmp_dir

  setup %{tmp_dir: tmp} do
    openssl = openssl_bin()
    ca_dir = Path.join(tmp, "ca")
    File.mkdir_p!(ca_dir)
    ca_key = Path.join(ca_dir, "ca-key.pem")
    ca_cert = Path.join(ca_dir, "ca-cert.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-x509",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          ca_key,
          "-out",
          ca_cert,
          "-days",
          "1",
          "-subj",
          "/CN=alchemy-test-client-ca"
        ],
        stderr_to_stdout: true
      )

    Application.put_env(:world, :mtls_ca_cert_path, ca_cert)
    Application.put_env(:world, :mtls_ca_key_path, ca_key)
    Application.put_env(:world, :mtls_work_dir, Path.join(tmp, "work"))

    on_exit(fn ->
      Application.delete_env(:world, :mtls_ca_cert_path)
      Application.delete_env(:world, :mtls_ca_key_path)
      Application.delete_env(:world, :mtls_ca_cert_pem)
      Application.delete_env(:world, :mtls_ca_key_pem)
      Application.delete_env(:world, :mtls_work_dir)
    end)

    Application.put_env(:world, :openssl_bin, openssl)
    %{ca_dir: ca_dir, tmp: tmp, openssl: openssl}
  end

  test "forces CN from role and ignores CSR subject", %{tmp: tmp, openssl: openssl} do
    csr = make_csr(tmp, "/CN=alchemy-core", openssl)

    assert {:ok, %{client_cert_pem: cert, ca_cert_pem: ca, expires_at: exp}} =
             MtlsIssuer.issue(csr, "guest")

    assert is_binary(ca) and String.contains?(ca, "BEGIN CERTIFICATE")
    assert String.contains?(cert, "BEGIN CERTIFICATE")
    assert %DateTime{} = exp

    subject = cert_subject(tmp, cert, openssl)
    assert subject =~ "CN = alchemy-guest" or subject =~ "CN=alchemy-guest"
    refute subject =~ "alchemy-core"
  end

  test "NotAfter and expires_at match TTL 300s", %{tmp: tmp, openssl: openssl} do
    csr = make_csr(tmp, "/CN=ignored", openssl)
    before = DateTime.utc_now() |> DateTime.truncate(:second)

    assert {:ok, %{client_cert_pem: cert, expires_at: exp}} =
             MtlsIssuer.issue(csr, "guest", ttl_seconds: 300)

    after_dt = DateTime.utc_now() |> DateTime.truncate(:second)
    soft_lo = DateTime.add(before, 300 - 5, :second)
    soft_hi = DateTime.add(after_dt, 300 + 5, :second)
    assert DateTime.compare(exp, soft_lo) != :lt
    assert DateTime.compare(exp, soft_hi) != :gt

    end_unix = cert_enddate_unix(tmp, cert, openssl)
    before_unix = DateTime.to_unix(before)
    after_unix = DateTime.to_unix(after_dt)
    assert end_unix >= before_unix + 300 - 30
    assert end_unix <= after_unix + 300 + 30
  end

  test "host role yields alchemy-host CN", %{tmp: tmp, openssl: openssl} do
    csr = make_csr(tmp, "/CN=ignored-host-claim", openssl)

    assert {:ok, %{client_cert_pem: cert}} = MtlsIssuer.issue(csr, "host")

    subject = cert_subject(tmp, cert, openssl)
    assert subject =~ "CN = alchemy-host" or subject =~ "CN=alchemy-host"
  end

  test "configured? false without CA" do
    Application.delete_env(:world, :mtls_ca_cert_path)
    Application.delete_env(:world, :mtls_ca_key_path)
    refute MtlsIssuer.configured?()
  end

  test "sign failure removes temp CA copies when CA is inline PEM", %{tmp: tmp} do
    ca_cert = Application.get_env(:world, :mtls_ca_cert_path)
    ca_key = Application.get_env(:world, :mtls_ca_key_path)
    cert_pem = File.read!(ca_cert)
    key_pem = File.read!(ca_key)

    Application.delete_env(:world, :mtls_ca_cert_path)
    Application.delete_env(:world, :mtls_ca_key_path)
    Application.put_env(:world, :mtls_ca_cert_pem, cert_pem)
    Application.put_env(:world, :mtls_ca_key_pem, key_pem)

    work = Path.join(tmp, "work")
    File.mkdir_p!(work)

    assert {:error, :openssl_failed} = MtlsIssuer.issue("not-a-real-csr", "guest")

    leftover =
      File.ls!(work)
      |> Enum.filter(&(String.starts_with?(&1, "ca-") or String.starts_with?(&1, "req-")))

    assert leftover == []
  end

  defp openssl_bin do
    found =
      [
        System.get_env("OPENSSL_BIN"),
        "openssl",
        "C:/Program Files/Git/usr/bin/openssl.exe",
        "C:/Program Files/OpenSSL-Win64/bin/openssl.exe"
      ]
      |> Enum.reject(&is_nil/1)
      |> Enum.find(&openssl_ok?/1)

    found || flunk("openssl not found (set OPENSSL_BIN)")
  end

  defp openssl_ok?(bin) do
    match?({_, 0}, System.cmd(bin, ["version"], stderr_to_stdout: true))
  rescue
    _ -> false
  end

  defp make_csr(tmp, subj, openssl) do
    key = Path.join(tmp, "client.key.pem")
    csr_path = Path.join(tmp, "client.csr.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          key,
          "-out",
          csr_path,
          "-subj",
          subj
        ],
        stderr_to_stdout: true
      )

    File.read!(csr_path)
  end

  defp cert_subject(tmp, cert_pem, openssl) do
    path = Path.join(tmp, "issued.cert.pem")
    File.write!(path, cert_pem)

    {subject, 0} =
      System.cmd(openssl, ["x509", "-in", path, "-noout", "-subject"], stderr_to_stdout: true)

    subject
  end

  defp cert_enddate_unix(tmp, cert_pem, openssl) do
    path = Path.join(tmp, "issued-end.cert.pem")
    File.write!(path, cert_pem)

    {out, 0} =
      System.cmd(openssl, ["x509", "-in", path, "-noout", "-enddate"], stderr_to_stdout: true)

    # notAfter=Nov  1 14:45:03 2026 GMT
    "notAfter=" <> rest = String.trim(out)

    rest =
      rest
      |> String.replace_suffix(" GMT", "")
      |> String.replace_suffix(" UTC", "")
      |> String.trim()

    [mon_s, day_s, time_s, year_s] = String.split(rest)

    mon =
      %{
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12
      }[mon_s]

    [hh, mm, ss] = String.split(time_s, ":") |> Enum.map(&String.to_integer/1)
    y = String.to_integer(year_s)
    d = String.to_integer(day_s)

    greg =
      :calendar.datetime_to_gregorian_seconds({{y, mon, d}, {hh, mm, ss}})

    # Unix epoch = 1970-01-01 in gregorian seconds
    greg - 62_167_219_200
  end
end
