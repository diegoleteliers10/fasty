cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.22.3"
  sha256 arm:   "8d2d4653105d22acbdba8660ea0c5639285d2d6ee57b6f475d777299ae9286b8",
         intel: "699b1811c7adce0b025194c9f301edd078fbea0c38651baea58f24a840b6a289"

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
