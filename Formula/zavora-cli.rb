class ZavoraCli < Formula
  desc "Rust CLI agent shell built on ADK-Rust"
  homepage "https://github.com/zavora-ai/zavora-cli"
  url "https://github.com/zavora-ai/zavora-cli/archive/refs/tags/v1.1.4.tar.gz"
  sha256 "7859ee635ed70ab33398ca3c6f8db86fe48ae22ea12dd80871e710a60a0d9299"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", *std_cargo_args(path: ".")
  end

  test do
    assert_match "Usage:", shell_output("#{bin}/zavora-cli --help")
  end
end
