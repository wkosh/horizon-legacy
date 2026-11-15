#!/bin/bash
# Packages Horizon Legacy's releases as the launcher reads them (docs/legacy-servers.md, "Releases").
#
# Every update is published twice, once per line: `v<N>.0.0-1.18.2` ("Legacy v<N> · 1.18.2") and
# `v<N>.0.0-1.17.1` ("Legacy v<N> · 1.17.1"). Both carry the same game, which joins servers of either
# release; the launcher tells the game its line (HORIZON_LEGACY_TARGET) and updates a version only to a
# newer one of its own line. Each release folder holds upstream's asset names (horizon-linux-x86_64.tar.gz,
# horizon-windows-x64.zip, one top folder each) and SHA256SUMS + SHA256SUMS.sig, signed by
# tools/ci/sign_release.sh with RELEASE_SIGNING_KEY. The archives carry no precompiled exporters (they need
# Mojang's jars): horizon-prepare compiles them from the Java sources archived here, with a JDK the launcher
# finds or downloads.
#
# With LAUNCHER_VERSION set, the launcher's release `launcher@<version>` too: the binary alone in
# horizon-launcher-linux-x86_64.tar.gz and horizon-launcher-windows-x64.zip (what it updates itself from),
# and the Windows binary as horizon-launcher-setup.exe and horizon-legacy-setup.exe (the per-user installer).
#
# Build first, with HORIZON_VERSION=<N>.0.0 for the game and HORIZON_LAUNCHER_VERSION / HORIZON_LAUNCHER_TAG
# for the launcher: cargo build --release (Linux) and --target x86_64-pc-windows-gnu (Windows; the game with
# RUSTFLAGS="-C target-feature=+crt-static").
#   RELEASE_SIGNING_KEY=$(cat key.txt) LAUNCHER_VERSION=1.0.0 tools/ci/package-legacy.sh 1 dist
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
major=${1:?usage: package-legacy.sh MAJOR OUT_DIR}
out=${2:?usage: package-legacy.sh MAJOR OUT_DIR}
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The game, once per platform.
archives=()
for plat in linux-x86_64 windows-x64; do
  case $plat in
    linux-x86_64) bin=$repo/target/release; exe=; asset=horizon-linux-x86_64.tar.gz ;;
    windows-x64) bin=$repo/target/x86_64-pc-windows-gnu/release; exe=.exe; asset=horizon-windows-x64.zip ;;
  esac
  [[ -f $bin/horizon-official$exe ]] || { echo "skip the game for $plat: not built"; continue; }
  dir=horizon-$plat
  mkdir -p "$work/$dir"
  git -C "$repo" archive --format=tar HEAD tools mods/horizon/src/test/java crates/horizon-prepare research-lock.json LICENSE | tar -x -C "$work/$dir"
  cp "$bin/horizon-official$exe" "$work/$dir/horizon-client$exe"
  for tool in horizon-ctl horizon-prepare horizon-server; do cp "$bin/$tool$exe" "$work/$dir/"; done
  cp "$repo/packaging/PLAY.txt" "$work/$dir/PLAY.txt"
  cp "$repo/packaging/LEGACY.txt" "$work/$dir/LEGACY.txt"
  if [[ $asset == *.zip ]]; then (cd "$work" && zip -qr "$asset" "$dir"); else (cd "$work" && tar czf "$asset" "$dir"); fi
  archives+=("$asset")
done

for line in 1.18.2 1.17.1; do
  [[ ${#archives[@]} -gt 0 ]] || break
  tag=v$major.0.0-$line
  rm -rf "${out:?}/$tag" && mkdir -p "$out/$tag"
  files=()
  for asset in "${archives[@]}"; do cp "$work/$asset" "$out/$tag/"; files+=("$out/$tag/$asset"); done
  bash "$repo/tools/ci/sign_release.sh" "$tag" "${files[@]}"
  cat > "$out/$tag/RELEASE.txt" <<EOF
Tag: $tag
Title: Legacy v$major · $line
Pre-release: no (the launcher installs only releases)
Files: ${archives[*]} SHA256SUMS SHA256SUMS.sig
EOF
done

if [[ -n "${LAUNCHER_VERSION:-}" ]]; then
  tag=launcher@$LAUNCHER_VERSION
  rm -rf "${out:?}/$tag" && mkdir -p "$out/$tag"
  files=()
  linux=$repo/launcher/target/release/horizon-launcher
  windows=$repo/launcher/target/x86_64-pc-windows-gnu/release/horizon-launcher.exe
  if [[ -f $linux ]]; then
    (cd "$(dirname "$linux")" && tar czf "$out/$tag/horizon-launcher-linux-x86_64.tar.gz" horizon-launcher)
    files+=("$out/$tag/horizon-launcher-linux-x86_64.tar.gz")
  fi
  if [[ -f $windows ]]; then
    (cd "$(dirname "$windows")" && rm -f "$out/$tag/horizon-launcher-windows-x64.zip" && zip -q "$out/$tag/horizon-launcher-windows-x64.zip" horizon-launcher.exe)
    cp "$windows" "$out/$tag/horizon-launcher-setup.exe"
    cp "$windows" "$out/$tag/horizon-legacy-setup.exe"
    files+=("$out/$tag/horizon-launcher-windows-x64.zip" "$out/$tag/horizon-launcher-setup.exe" "$out/$tag/horizon-legacy-setup.exe")
  fi
  if [[ ${#files[@]} -gt 0 ]]; then
    bash "$repo/tools/ci/sign_release.sh" "$tag" "${files[@]}"
    cat > "$out/$tag/RELEASE.txt" <<EOF
Tag: $tag
Title: Horizon Legacy Launcher $LAUNCHER_VERSION
Pre-release: no, and not "latest" (the game's releases are)
Files: $(cd "$out/$tag" && ls | grep -v RELEASE.txt | tr '\n' ' ')
EOF
  fi
fi
ls -R "$out"
