cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.1"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000",
         intel: "0000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/diegoleteliers10/fasty/releases/download/v#{version}/fastty-#{arch}-apple-darwin.dmg"
  name "Fastty"
  desc "Fast, GPU-accelerated terminal emulator built with Rust & GPUI"
  homepage "https://github.com/diegoleteliers10/fasty"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :sierra"

  app "Fastty.app"

  # Fastty is not (yet) signed/notarized by an Apple Developer ID, so a
  # freshly downloaded copy gets Gatekeeper's quarantine flag and macOS
  # would otherwise refuse to open it ("Fastty is damaged and can't be
  # opened"). This mirrors what instalar.sh already does for direct
  # installs.
  postflight do
    system_command "/usr/bin/xattr",
                    args: ["-dr", "com.apple.quarantine", "#{appdir}/Fastty.app"],
                    sudo: false
  end

  zap trash: [
    "~/Library/Application Support/fastty",
    "~/Library/Caches/fastty",
    "~/Library/Preferences/com.diegoleteliers10.fastty.plist",
    "~/Library/Saved Application State/com.diegoleteliers10.fastty.savedState",
  ]
end
