defmodule World.Domain do
  @moduledoc """
  Ash domain for world-server room ledger (x0028).
  """

  use Ash.Domain,
    otp_app: :world

  resources do
    resource World.Rooms.Room
  end
end
