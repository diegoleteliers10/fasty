cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.11.0"
  sha256 arm:   "08f5e72c8ff5119dda166ad804a147fe48066b917f4ed87ac9377d1b26d34034",
         intel: "27efdb71a8c587d62ccd900def54b19504ccfa28102ef3aa8a92d4de518c5d0c"

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

  # Fastty is ad-hoc signed. The postflight removes quarantine attributes
  # and refreshes the ad-hoc signature to prevent Gatekeeper damage alerts.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-cr", "#{appdir}/Fastty.app"],
                   sudo: false
    system_command "/usr/bin/codesign",
                   args: ["--force", "--deep", "-s", "-", "#{appdir}/Fastty.app"],
                   sudo: false
  end

  zap trash: [
    "~/Library/Application Support/fastty",
    "~/Library/Caches/fastty",
    "~/Library/Preferences/com.diegoleteliers10.fastty.plist",
    "~/Library/Saved Application State/com.diegoleteliers10.fastty.savedState",
  ]
end
