cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.10.1"
  sha256 arm:   "103520193e9e60c491314089236a430bb8a938139d1d69342c36c1bb4c7ef127",
         intel: "6ed38fa915ac896839faea98f57e69fdec3beb75d49c005d568e0050e12add1f"

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
