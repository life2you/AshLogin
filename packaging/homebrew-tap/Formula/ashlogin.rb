class Ashlogin < Formula
  desc "Terminal-first SSH account manager and login launcher for macOS"
  homepage "https://github.com/life2you/AshLogin"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/life2you/AshLogin/releases/download/v0.1.0/ashlogin-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    end

    on_intel do
      url "https://github.com/life2you/AshLogin/releases/download/v0.1.0/ashlogin-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X64_SHA256"
    end
  end

  def install
    bin.install "ashlogin"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ashlogin --version")
  end
end
