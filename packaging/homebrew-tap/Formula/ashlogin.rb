class Ashlogin < Formula
  desc "Terminal-first SSH account manager and login launcher for macOS"
  homepage "https://github.com/life2you/AshLogin"
  url "https://github.com/life2you/AshLogin/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "d07628f4196c2bba77fe3736bac5f374368e08ccf1f456733f61ea4f80955c90"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ashlogin --version")
  end
end
