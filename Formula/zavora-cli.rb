class ZavoraCli < Formula
  desc "Rust CLI agent shell built on ADK-Rust"
  homepage "https://github.com/zavora-ai/zavora-cli"
  url "https://github.com/zavora-ai/zavora-cli/archive/refs/tags/v2.0.0.tar.gz"
  sha256 "52783fa4ab0e0db3f62ab13f3a1746608029b2d50f681908e9553a83ffc1e2d7"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", *std_cargo_args(path: ".")
  end

  test do
    assert_match "Usage:", shell_output("#{bin}/zavora-cli --help")
  end
end
