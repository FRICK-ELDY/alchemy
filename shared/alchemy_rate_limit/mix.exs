defmodule AlchemyRateLimit.MixProject do
  use Mix.Project

  def project do
    [
      app: :alchemy_rate_limit,
      version: "0.1.0",
      elixir: "~> 1.19",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      package: package(),
      description: "ETS fixed-window rate limiter for Alchemy auth/world (x0071)"
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
      {:telemetry, "~> 1.0"}
    ]
  end

  defp package do
    [
      licenses: ["MPL-2.0"],
      links: %{"GitHub" => "https://github.com/FRICK-ELDY/alchemy"}
    ]
  end
end
