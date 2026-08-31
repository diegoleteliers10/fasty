cask "fastty" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.5"
  sha256 arm:   "8b0bf7d1d6b3b3f7c11cd6f07eb204fe91984fdd5b3602858b5377206d1a4ba6",
         intel: "3d497bd23950a10830615df35487ff4d9bd721ff7e4f887b0ce7c6a8ab880e13"

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
