cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.22.4"
  sha256 arm:   "de9c13ef10cac7bbb58690242001cebc956990b7d1257e657ac362ba815b6dd6",
         intel: "fd213f26488972986df91234c8be10b03fe45d82b34c214dde6873d630f3c806"

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
