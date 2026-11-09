cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.28.2"
  sha256 arm:   "024b78e434b2b3f64aea05b4e56d958caf5f7aa982a8eb871507029d3b25146a",
         intel: "3f7669a7c51c421d27eb73bdaf3de776dd8f4ef0427f93aeb950c35c4e84bda3"

  url "https://github.com/diegoleteliers10/fasty/releases/download/v#{version}/fastty-#{arch}-apple-darwin.dmg"
  name "Fastty"
  desc "Fast, GPU-accelerated terminal emulator built with Rust & GPUI"
  homepage "https://github.com/diegoleteliers10/fasty"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true

  app "Fastty.app"
  binary "#{appdir}/Fastty.app/Contents/MacOS/fastty"

  # The release bundle already carries its signature. The postflight only
  # removes the quarantine attribute that triggers the Gatekeeper prompt.
  # Re-signing here would replace a stable signing identity with an ad-hoc one,
  # whose content hash changes on every build, and macOS would then ask the user
  # to accept the app again after every update.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-cr", "#{appdir}/Fastty.app"],
                   sudo: false
  end

  zap trash: [
    "~/Library/Application Support/fastty",
    "~/Library/Caches/fastty",
    "~/Library/Preferences/com.diegoleteliers10.fastty.plist",
    "~/Library/Saved Application State/com.diegoleteliers10.fastty.savedState",
  ]
end
