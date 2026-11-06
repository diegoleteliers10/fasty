cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.26.0"
  sha256 arm:   "d9912955bc7d5cc60ecc42a9ef34b586a606b1e69e600e0bb78381ea8fd0618d",
         intel: "8f3c8878e5abedbdabf5b834bce43355a791b0b083ccd551812f7f68f8e81ef7"

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
