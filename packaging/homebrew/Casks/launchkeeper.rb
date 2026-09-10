# Cask for the desktop app: the signed, notarized dmg from the GitHub
# release. Unlike the Formula this is *not* built locally — the whole point of
# shipping a dmg is that it carries a Developer ID signature, which is what
# lets macOS keep the runner's TCC grants across updates.
#
# `scripts/release/update-tap.sh` rewrites `version`, `sha256` and the owner
# in this file; the owner comes from packaging/homebrew/repo.env.
cask "launchkeeper" do
  version "0.1.0"
  sha256 "4077a632a664cea0889b2677136237f75b02ae113bbdaf4ed6d07855d4d9b239"

  url "https://github.com/code-better-life/launchkeeper/releases/download/v#{version}/Launchkeeper_#{version}_aarch64.dmg"
  name "Launchkeeper"
  desc "Visual manager for launchd automation tasks"
  homepage "https://github.com/code-better-life/launchkeeper"

  # Only an aarch64 dmg is published; the Formula covers everything else.
  depends_on arch: :arm64
  # Matches bundle.macOS.minimumSystemVersion in tauri.conf.json.
  depends_on macos: :ventura

  app "Launchkeeper.app"
  # The same `launchkeeper` binary the Formula installs, signed with the app,
  # so cask-only users get the CLI too. If the Formula is also installed it
  # already owns that name in the prefix; Homebrew then warns and leaves the
  # Formula's copy alone rather than failing the install.
  binary "#{appdir}/Launchkeeper.app/Contents/MacOS/launchkeeper"

  uninstall quit: "com.launchkeeper.app"

  # Uninstalling the app must not silently keep running the user's scripts:
  # bootout every job Launchkeeper created before removing its data. The
  # LaunchAgents glob is the same `com.launchkeeper.` prefix the app is only
  # ever allowed to write, so nothing hand-written is touched. Jobs adopted
  # in place keep their original label and are deliberately left alone —
  # they existed before Launchkeeper and outlive it; `launchkeeper unadopt`
  # restores those from their .bak.
  zap launchctl: "com.launchkeeper.*",
      trash:     [
        "~/.config/launchkeeper",
        "~/Library/Application Support/Launchkeeper",
        "~/Library/Caches/com.launchkeeper.app",
        "~/Library/HTTPStorages/com.launchkeeper.app",
        "~/Library/LaunchAgents/com.launchkeeper.*.plist",
        "~/Library/Logs/Launchkeeper",
        "~/Library/Preferences/com.launchkeeper.app.plist",
        "~/Library/Saved Application State/com.launchkeeper.app.savedState",
        "~/Library/WebKit/com.launchkeeper.app",
      ]
end
