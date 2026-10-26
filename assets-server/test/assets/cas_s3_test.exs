defmodule Assets.CAS.S3Test do
  use ExUnit.Case, async: true

  alias Assets.CAS.S3

  setup do
    previous = Application.get_env(:assets, :cas_s3)

    Application.put_env(:assets, :cas_s3, %{
      bucket: "test-bucket",
      host: "example.r2.cloudflarestorage.com",
      access_key_id: "id",
      secret_access_key: "secret",
      region: "auto",
      prefix: "cas/"
    })

    on_exit(fn ->
      if previous do
        Application.put_env(:assets, :cas_s3, previous)
      else
        Application.delete_env(:assets, :cas_s3)
      end
    end)

    :ok
  end

  test "object_key joins prefix and blake3" do
    assert S3.object_key("abc") == "cas/abc"
  end
end
