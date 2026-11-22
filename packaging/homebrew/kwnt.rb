class Kwnt < Formula
  desc "Self-hosted team of AI bots with handoffs, approvals and routines"
  homepage "https://kwant.si"
  url "https://github.com/alexsergivan/kwant/archive/refs/tags/mac-v0.9.17.tar.gz"
  sha256 "28aa19ddb26195f33bb6854b6d7d258e0696ce3fe0846fe875b7d785ac9611c4"
  license "AGPL-3.0-only"
  head "https://github.com/alexsergivan/kwant.git", branch: "main"

  livecheck do
    url :stable
    regex(/^mac-v?(\d+(?:\.\d+)+)$/i)
  end

  depends_on "node" => :build
  depends_on "rust" => :build

  on_linux do
    depends_on "pkgconf" => :build
    depends_on "openssl@4"
  end

  def install
    # The kwnt binary embeds the web UI from frontend/dist, so it is built first
    cd "frontend" do
      system "npm", "install", *std_npm_args(prefix: false)
      system "npm", "run", "build"
    end
    # Homebrew updates kwnt: the build never looks for a new release (as with KWNT_UPDATE_CHECK=off)
    system "cargo", "install", *std_cargo_args(features: "no-update-check")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/kwnt --version")

    ENV["KWNT_DATA_DIR"] = (testpath/"data").to_s
    ENV["KWNT_LAYA_MANAGED"] = "0"
    assert_match "not created yet", shell_output("#{bin}/kwnt status")

    port = free_port
    pid = spawn bin/"kwnt", "serve", "--host", "127.0.0.1", "--port", port.to_s, "--no-laya"
    begin
      health = ""
      30.times do
        sleep 1
        health = Utils.popen_read("curl", "-s", "http://127.0.0.1:#{port}/api/health")
        break if health.include?('"app":"kwnt"')
      end
      assert_match '"app":"kwnt"', health
      assert_match version.to_s, health
      # Built with no-update-check: the server reports updates off without KWNT_UPDATE_CHECK
      update = shell_output("curl -s http://127.0.0.1:#{port}/api/app/update")
      assert_match '"supported":false', update
    ensure
      Process.kill("TERM", pid)
      Process.wait(pid)
    end
  end
end
