# Formula for the command-line half of Launchkeeper: the `launchkeeper` CLI
# and the `launchkeeper-runner` helper it writes into every plist.
#
# Built from the source tarball with cargo rather than shipping a bottle, so
# that the runner is compiled and ad-hoc signed on the user's own machine —
# the CLI half has no Developer ID certificate behind it and a downloaded,
# unsigned Mach-O would be quarantined.
#
# The two binaries deliberately land in the same `bin/`: that sibling layout
# is the first thing `resolve_runner_source` probes, so `launchkeeper add …`
# finds the runner with no configuration at all.
#
# The desktop app is a separate artifact — see ../Casks/launchkeeper.rb.
#
# `scripts/release/update-tap.sh` rewrites `url`, `sha256` and the owner in
# this file; the owner it writes comes from packaging/homebrew/repo.env.
class Launchkeeper < Formula
  desc "Visual manager for launchd automation tasks (CLI)"
  homepage "https://github.com/code-better-life/launchkeeper"
  url "https://github.com/code-better-life/launchkeeper/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "9531bd98b8697e22724605cc36c0855bfa209a08596d58936674d065cb192f9f"
  license "MIT"
  head "https://github.com/code-better-life/launchkeeper.git", branch: "main"

  depends_on "rust" => :build
  # launchd LaunchAgents, `launchctl bootstrap gui/$UID`, TCC: none of this
  # has a meaning off macOS.
  depends_on macos: :ventura

  def install
    # Two `cargo install` calls rather than one `cargo build`: `std_cargo_args`
    # carries Homebrew's `--locked` and `--root`, which keep the build
    # reproducible and put the binaries straight into the keg's bin/.
    system "cargo", "install", *std_cargo_args(path: "crates/launchkeeper-cli")
    system "cargo", "install", *std_cargo_args(path: "crates/launchkeeper-runner")

    # `launchkeeper completions <shell>` prints the static script. Generating
    # it from the binary that was just built means the completions can never
    # describe flags this version does not have.
    generate_completions_from_executable(bin/"launchkeeper", "completions")
  end

  def caveats
    <<~EOS
      Launchkeeper writes LaunchAgents into ~/Library/LaunchAgents and keeps its
      database under ~/Library/Application Support/Launchkeeper (override with
      LAUNCHKEEPER_DATA_DIR, or persistently with `launchkeeper config data-dir`).

      launchd is the only scheduler: nothing runs in the background on
      Launchkeeper's behalf, and `brew uninstall` leaves already-scheduled jobs
      alone. Remove them first with `launchkeeper rm <name>` if that is not what
      you want.

      The desktop app is a separate cask:
        brew install --cask code-better-life/launchkeeper/launchkeeper
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/launchkeeper --version")

    # The runner must be found without --runner or LAUNCHKEEPER_RUNNER: that
    # sibling lookup is the whole reason both binaries go into the same bin/.
    assert_path_exists bin/"launchkeeper-runner"

    # An empty data dir in the sandbox: proves the store is created and the
    # machine-readable contract holds, without touching the real one. All
    # three overrides are mandatory together — with only the data dir set,
    # the status lookup still reads the real ~/Library/LaunchAgents.
    ENV["LAUNCHKEEPER_DATA_DIR"] = testpath/"data"
    ENV["LAUNCHKEEPER_LAUNCH_AGENTS_DIR"] = testpath/"agents"
    ENV["LAUNCHKEEPER_RUNNER_LOG_DIR"] = testpath/"logs"
    assert_equal "[]", shell_output("#{bin}/launchkeeper list --json").strip
  end
end
