cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.15.0"
  sha256 arm:   "8c9bc3e45b47e0e4fb7ba7708d5d830b6c096991e86acaa0b333a244fe813dc7",
         intel: "ad9aabdea0b5d56b91b1af541907812f516c7b07ce18846cd752cf27e13f03b6"

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
