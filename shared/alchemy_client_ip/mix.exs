defmodule AlchemyClientIp.MixProject do
  use Mix.Project

  def project do
    [
      app: :alchemy_client_ip,
      version: "0.1.0",
      elixir: "~> 1.19",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      package: package(),
      description: "Trusted-proxy client IP (CF-Connecting-IP / XFF) for Alchemy auth/world"
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp elixirc_paths(_), do: ["lib"]

  defp deps do
    [
      {:plug, "~> 1.16"},
      {:remote_ip, "~> 1.2"}
    ]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => "https://github.com/FRICK-ELDY/alchemy"}
    ]
  end
end
