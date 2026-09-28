cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.14.9"
  sha256 arm:   "2529b0ca41ce8269640b198118c70db7ef13812e02a1dbfd0b3d649345a64649",
         intel: "80dcef44fa472b2d5068e1098a9c685a6a545fe4b04e1952bf0017e34f0da87e"

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
