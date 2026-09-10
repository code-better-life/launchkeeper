#!/usr/bin/env bash
# Builds the release .app and .dmg.
#
#   scripts/release/build-dmg.sh
#
# Order matters: the CLI and the runner are built *first*, because
# `prepare-runner.mjs` copies them out of `target/release/` into
# `src-tauri/binaries/` where Tauri's `externalBin` picks them up, and only
# then does `tauri build` produce the bundle. Both helpers therefore end up
# inside `Launchkeeper.app/Contents/MacOS/`, signed by whatever identity
# signed the app.
#
# Signing:
#   - `APPLE_SIGNING_IDENTITY` set  -> Tauri signs the app and its helpers
#     with that Developer ID and the hardened runtime, and builds the dmg
#     around the signed app. Notarization is a separate step (see
#     `docs/private/RELEASING.md` and `.github/workflows/release.yml`).
#   - unset (every local build on a machine with no certificate) -> the
#     bundle is sealed ad hoc afterwards and the dmg is rebuilt around it,
#     so the artifact is structurally identical to the signed one and can be
#     mounted, dragged to /Applications and launched (after the usual
#     right-click > Open, since ad-hoc signatures are not notarized).
#
# Everything is written under the repo's own `target/`; nothing is installed.
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo_root"

app_dir="$repo_root/crates/launchkeeper-app"
bundle_conf="src-tauri/tauri.bundle.conf.json"
product="Launchkeeper"

log() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror: %s\033[0m\n' "$*" >&2; exit 1; }

for tool in cargo node pnpm rustc; do
  command -v "$tool" >/dev/null 2>&1 || die "找不到 $tool，先把工具链放进 PATH"
done

version=$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f && /^version[[:space:]]*=/{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)
[ -n "$version" ] || die "无法从 Cargo.toml 的 [workspace.package] 解析 version"
arch=$(uname -m)
log "Launchkeeper $version ($arch)"

log "构建 CLI 与 runner (release)"
cargo build --release --bin launchkeeper --bin launchkeeper-runner

if [ ! -d "$app_dir/node_modules" ]; then
  log "安装前端依赖"
  (cd "$app_dir" && pnpm install --frozen-lockfile)
fi

log "把辅助二进制放进 src-tauri/binaries/"
(cd "$app_dir" && node scripts/prepare-runner.mjs --profile release)

log "打包 app + dmg"
# The overlay config adds `externalBin`; it is kept out of the main config so
# that `pnpm tauri dev` does not demand the copied binaries. See
# crates/launchkeeper-app/scripts/prepare-runner.mjs.
(cd "$app_dir" && pnpm tauri build --config "$bundle_conf")

bundle_dir="$repo_root/target/release/bundle"
app="$bundle_dir/macos/$product.app"
[ -d "$app" ] || die "没有产出 $app"
dmg=$(ls -t "$bundle_dir"/dmg/*.dmg 2>/dev/null | head -1 || true)
[ -n "$dmg" ] || die "没有产出 dmg（看 $bundle_dir/dmg/）"

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  log "已用 Developer ID 签名：$APPLE_SIGNING_IDENTITY"
else
  log "没有 APPLE_SIGNING_IDENTITY，改为 ad-hoc 签名并重建 dmg"
  # Inner binaries first, then the bundle: signing the outside seals the
  # inside, so the reverse order invalidates the app's own signature.
  for helper in launchkeeper-runner launchkeeper; do
    [ -e "$app/Contents/MacOS/$helper" ] &&
      codesign --force --sign - "$app/Contents/MacOS/$helper"
  done
  codesign --force --sign - "$app"

  staging=$(mktemp -d)
  trap 'rm -rf "$staging"' EXIT
  cp -R "$app" "$staging/"
  ln -s /Applications "$staging/Applications"
  rm -f "$dmg"
  hdiutil create -volname "$product" -srcfolder "$staging" \
    -ov -format UDZO -quiet "$dmg"
fi

codesign --verify --deep --strict "$app" ||
  die "codesign --verify 失败：$app"

log "产出"
for f in "$app" "$dmg"; do
  printf '%-14s %s\n' "$(du -sh "$f" | cut -f1)" "$f"
done
printf '\nsha256:\n'
shasum -a 256 "$dmg"
