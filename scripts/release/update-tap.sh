#!/usr/bin/env bash
# Rewrites packaging/homebrew/{Formula,Casks}/launchkeeper.rb for a release,
# and optionally pushes them to the tap.
#
#   scripts/release/update-tap.sh <version> [options]
#
#   --dmg-sha256 <hex>   sha256 of Launchkeeper_<version>_aarch64.dmg
#   --src-sha256 <hex>   sha256 of the v<version> source tarball
#   --push               clone $TAP_REPO with $TAP_TOKEN, commit, push
#   --dry-run            print the rewritten files, change nothing
#
# Either checksum may be omitted, in which case it is downloaded from the
# GitHub release / the tag's source tarball and hashed. That is what CI does
# after uploading the artifacts, and what a human does when re-cutting a tap
# for an already-published release.
#
# Owner/repo live in exactly one place: packaging/homebrew/repo.env. This
# script rewrites every URL in both .rb files from there, so a fork only has
# to edit that file.
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo_root"

# shellcheck source=../../packaging/homebrew/repo.env
. packaging/homebrew/repo.env

version=""
dmg_sha=""
src_sha=""
push=0
dry_run=0

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dmg-sha256) dmg_sha="$2"; shift 2 ;;
    --src-sha256) src_sha="$2"; shift 2 ;;
    --push) push=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    -*) die "未知参数 $1" ;;
    *) [ -z "$version" ] || die "只接受一个版本号"; version="$1"; shift ;;
  esac
done

[ -n "$version" ] || die "用法: $0 <version> [--dmg-sha256 …] [--src-sha256 …] [--push]"
version="${version#v}"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) die "版本号形如 1.2.3，收到 $version" ;;
esac

base="https://github.com/$RELEASE_REPO"
dmg_url="$base/releases/download/v$version/Launchkeeper_${version}_aarch64.dmg"
src_url="$base/archive/refs/tags/v$version.tar.gz"

# Downloads $1 and prints its sha256. Kept separate so the two "maybe fetch"
# branches below read the same.
sha_of_url() {
  local url="$1" tmp
  tmp=$(mktemp)
  curl -fsSL --retry 3 -o "$tmp" "$url" || { rm -f "$tmp"; die "下载失败: $url"; }
  shasum -a 256 "$tmp" | cut -d' ' -f1
  rm -f "$tmp"
}

[ -n "$dmg_sha" ] || { echo "==> 取 dmg sha256: $dmg_url" >&2; dmg_sha=$(sha_of_url "$dmg_url"); }
[ -n "$src_sha" ] || { echo "==> 取源码 sha256: $src_url" >&2; src_sha=$(sha_of_url "$src_url"); }

for h in "$dmg_sha" "$src_sha"; do
  printf '%s' "$h" | grep -qE '^[0-9a-f]{64}$' || die "不是 sha256: $h"
done

formula="packaging/homebrew/Formula/launchkeeper.rb"
cask="packaging/homebrew/Casks/launchkeeper.rb"

# What a user types: `brew tap code-better-life/launchkeeper` drops the
# `homebrew-` prefix the repository name has to carry.
tap_short="${TAP_REPO%%/*}/${TAP_REPO#*/homebrew-}"
# The cask keeps `#{version}` in its url so that only the `version` line has
# to change next time. `#` is not a comment inside double quotes.
cask_url="$base/releases/download/v#{version}/Launchkeeper_#{version}_aarch64.dmg"

# `|` as the sed delimiter throughout: every replacement contains slashes.
rewrite_formula() {
  sed \
    -e "s|^\( *homepage \).*|\1\"$base\"|" \
    -e "s|^\( *url \).*|\1\"$src_url\"|" \
    -e "s|^\( *sha256 \).*|\1\"$src_sha\"|" \
    -e "s|^\( *head \)\"[^\"]*\"\(.*\)|\1\"$base.git\"\2|" \
    -e "s|^\( *brew install --cask \).*|\1$tap_short/launchkeeper|" \
    "$formula"
}

rewrite_cask() {
  sed \
    -e "s|^\( *version \).*|\1\"$version\"|" \
    -e "s|^\( *sha256 \).*|\1\"$dmg_sha\"|" \
    -e "s|^\( *url \).*|\1\"$cask_url\",|" \
    -e "s|^\( *verified: \).*|\1\"github.com/$RELEASE_REPO/\"|" \
    -e "s|^\( *homepage \).*|\1\"$base\"|" \
    "$cask"
}

if [ "$dry_run" = 1 ]; then
  echo "--- $formula"; rewrite_formula
  echo "--- $cask"; rewrite_cask
  exit 0
fi

tmp=$(mktemp); rewrite_formula >"$tmp"; mv "$tmp" "$formula"
tmp=$(mktemp); rewrite_cask >"$tmp"; mv "$tmp" "$cask"
echo "已更新 $formula 和 $cask 到 v$version"

[ "$push" = 1 ] || exit 0

[ -n "${TAP_TOKEN:-}" ] || die "--push 需要 TAP_TOKEN（对 $TAP_REPO 有写权限的 token）"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git clone --depth 1 "https://x-access-token:$TAP_TOKEN@github.com/$TAP_REPO.git" "$work/tap"
mkdir -p "$work/tap/Formula" "$work/tap/Casks"
cp "$formula" "$work/tap/Formula/launchkeeper.rb"
cp "$cask" "$work/tap/Casks/launchkeeper.rb"
git -C "$work/tap" add Formula/launchkeeper.rb Casks/launchkeeper.rb
if git -C "$work/tap" diff --cached --quiet; then
  echo "tap 已经是 v$version，无需提交"
  exit 0
fi
git -C "$work/tap" -c user.name="launchkeeper-release" \
  -c user.email="noreply@github.com" \
  commit -m "chore: launchkeeper $version"
git -C "$work/tap" push
echo "已推送到 $TAP_REPO"
