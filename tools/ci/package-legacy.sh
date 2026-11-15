#!/bin/bash
# Packages the fork's release builds as upstream's release folder does (xtask package), without the
# precompiled exporters, which need Mojang's jars to build: horizon-prepare compiles them on the
# player's machine from the Java sources archived here.
# Build first with HORIZON_VERSION set: cargo build --release (Linux) and --target x86_64-pc-windows-gnu
# with RUSTFLAGS="-C target-feature=+crt-static" (Windows), for horizon-official, horizon-ctl,
# horizon-prepare and horizon-server (docs/legacy-servers.md, "Releases").
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
ver=${1:?usage: package-legacy.sh VERSION OUT_DIR}
out=${2:?usage: package-legacy.sh VERSION OUT_DIR}
mkdir -p "$out"
rm -f "$out/SHA256SUMS.txt"
for plat in linux-x86_64 windows-x64; do
  case $plat in
    linux-x86_64) bin=$repo/target/release; exe=; fmt=tar.gz ;;
    windows-x64) bin=$repo/target/x86_64-pc-windows-gnu/release; exe=.exe; fmt=zip ;;
  esac
  [[ -f $bin/horizon-official$exe ]] || { echo "skip $plat: not built"; continue; }
  dir=horizon-legacy-$ver-$plat
  rm -rf "$out/$dir" && mkdir -p "$out/$dir"
  git -C "$repo" archive --format=tar HEAD tools mods/horizon/src/test/java crates/horizon-prepare research-lock.json LICENSE | tar -x -C "$out/$dir"
  cp "$bin/horizon-official$exe" "$out/$dir/horizon-client$exe"
  for tool in horizon-ctl horizon-prepare horizon-server; do cp "$bin/$tool$exe" "$out/$dir/"; done
  cp "$repo/packaging/PLAY.txt" "$out/$dir/PLAY.txt"
  cp "$repo/packaging/LEGACY.txt" "$out/$dir/LEGACY.txt"
  (cd "$out" && rm -f "$dir.$fmt" && if [[ $fmt == zip ]]; then zip -qr "$dir.zip" "$dir"; else tar czf "$dir.tar.gz" "$dir"; fi)
  (cd "$out" && sha256sum "$dir.$fmt" >> SHA256SUMS.txt)
  echo "packaged $out/$dir.$fmt"
done
