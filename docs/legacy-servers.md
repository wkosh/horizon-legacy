# Joining 1.18.2 and 1.17.1 servers

This fork (`wkosh/horizon-legacy`) extends Horizon's native protocol adapter down to Minecraft 1.18.2
(protocol 758) and 1.17.1 (protocol 756). Everything is translated in Rust inside the connection: no proxy,
no ViaVersion, no Java. Upstream keeps the adapter off by default; in this fork it is on
(`HORIZON_OLDER_SERVERS=0` turns it off):

```sh
horizon-client --server play.example.org:25565
```

The server list marks 1.18.2 and 1.17.1 servers as joinable and the connection picks the
translation from the server's status ping.

## How it works

The adapter already speaks to servers from 26.2 down to 1.20.2 (764), one release at a time. Before 1.20.2
the wire is of an older kind, so the new layer, the **legacy front** (`crates/horizon-protocol/src/adapter/front/`),
sits between the socket and those steps and turns the old wire into what a 1.20.2 server would have sent,
registry ids included. Nothing above it knows the server is older.

| What differs before 1.20.2 | What the front does |
| --- | --- |
| No configuration state: `login success` puts the server in play, the play login carries the registry codec | Holds play packets until the login, then makes 1.20.2's configuration of it: registry data (dimension types and biomes brought to 1.20.2's fields, damage and chat types by name), enabled features, `finish_configuration`, then the play login |
| NBT with a root name | Copies each tag's bytes without the name (and adds it back to what Horizon sends) |
| Block states, items, entity types, particles, sounds, block entity types numbered by the old release | Renumbered through generated tables; a whole column's block states through one flat lookup table |
| Entity data serializers numbered differently, horse owner and cat type fields | Renumbered and moved |
| One spawn packet per kind of entity (mobs, paintings, players) | One `add_entity`; a painting's motive becomes its variant |
| One chat packet both ways, no signed chat | `system_chat` in; chat and commands out as the old packet |
| Block actions acknowledged by the block, not by sequence | The sequence is taken out and acknowledged at once |
| Player list as one packet with an action | 1.19.3's action set and `player_remove` |
| 1.17.1: chunks with a section mask and one biome array, light in a packet before the column, block entities as whole tags | Keeps the light until its column arrives, builds 1.18's sections and biome containers |
| Command tree parsers named by identifier | Numbered; the registry parsers 1.19.3 folded into `resource` name their registry |

The tables are generated from [PrismarineJS/minecraft-data](https://github.com/PrismarineJS/minecraft-data)
(MIT), so no Mojang file is read or committed:

```sh
python3 tools/generate_legacy_front.py --data path/to/minecraft-data/data
```

## Known gaps

- No parity harness covers these releases yet: the vanilla 1.18.2 and 1.17.1 jars are not part of Horizon's
  oracle setup. The translation is checked by unit tests against bytes laid out by hand and by a whole join
  through the adapter chain (`cargo test -p horizon-protocol --lib front`).
- Sounds 1.20.2 removed or renamed are dropped; particles 1.20.2 lacks are not drawn.
- Game event tags are not passed on (1.19 renamed them; only the server reads them).
- A recipe type unknown to 1.20.2 drops the whole recipe list, as any packet the front cannot read.

## Releases

Upstream's release workflows run on Horizon's own machines and sign with its keys, so the fork builds and signs
its releases itself, from Linux: Linux x86_64 and Windows x64 (cross-compiled with MinGW).

**One installer.** The fork's launcher (`launcher/`) is upstream's, pointed at this repository: it installs,
updates and starts the fork's builds, under its own name (Horizon Legacy) and data folders
(`horizon-legacy-launcher`, `horizon-legacy`), so it never mixes with an official install. Players download
`horizon-legacy-setup.exe` (Windows, the per-user installer) or the Linux archive from the newest
`launcher@X.Y.Z` release. Its update checks read only this repository (upstream's GitHub mirror is off), and it
installs only archives signed with the fork's key (`crates/horizon-release-sign/release-keys.txt`).

**Folders.** Nothing is shared with an official Horizon install; both can sit side by side:

| What | Windows | Linux |
| --- | --- | --- |
| The launcher (setup's default) | `%LOCALAPPDATA%\Programs\Horizon Legacy` | wherever the archive is unpacked |
| Versions, prepared resources, Java, the launcher's settings | `%APPDATA%\horizon-legacy-launcher` | `~/.local/share/horizon-legacy-launcher` |
| The game folder (worlds, options, logs) | `%APPDATA%\horizon-legacy` | `~/.local/share/horizon-legacy` |
| A moved data location's pointer | `%APPDATA%\horizon-legacy-launcher-location` | `~/.config/horizon-legacy-launcher-location` |
| Builds, for a user name that isn't ASCII | `C:\ProgramData\horizon-legacy-launcher-<id>` | |
| Shortcuts and registry | "Horizon Legacy" in the Start menu, `HKCU\Software\HorizonLegacy`, Uninstall `HorizonLegacyLauncher` | `horizon-legacy-launcher.desktop`, `horizon-legacy-instance-<id>.desktop` |

The game started by hand (not by the launcher) uses `horizon-legacy` too; `HORIZON_HOME` still overrides it.

**Versions.** Each update is published twice, `v<N>.0.0-1.18.2` and `v<N>.0.0-1.17.1`, which the launcher lists
as "Legacy v<N> · 1.18.2" and "Legacy v<N> · 1.17.1"; the next update is v<N+1>. Both carry the same game, which
joins servers of either release; the launcher passes the line to the game (`HORIZON_LEGACY_TARGET`, the release
a server whose status ping doesn't answer is joined as) and updates an installed version only to a newer one of
its own line.

**Windows launcher from Linux.** GPUI's Windows renderer compiles its HLSL with `fxc.exe` at build time, which
Linux lacks, so the launcher patches `gpui-fast-windows` (`launcher/vendor/gpui-fast-windows`, Apache-2.0) to
compile the same shaders when it starts, with Windows' own `d3dcompiler_47.dll`.

```sh
export HORIZON_VERSION=1.0.0 HORIZON_LAUNCHER_VERSION=1.0.0 HORIZON_LAUNCHER_TAG=launcher@1.0.0
cargo build --release --locked -p horizon-official --bin horizon-official -p horizon-client-lib --bin horizon-ctl \
  -p horizon-prepare --bin horizon-prepare -p horizon-server --bin horizon-server
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --locked --target x86_64-pc-windows-gnu \
  -p horizon-official --bin horizon-official -p horizon-client-lib --bin horizon-ctl \
  -p horizon-prepare --bin horizon-prepare -p horizon-server --bin horizon-server
(cd launcher && cargo build --release --locked && cargo build --release --locked --target x86_64-pc-windows-gnu)
RELEASE_SIGNING_KEY=$(cat signing-key.txt) LAUNCHER_VERSION=1.0.0 tools/ci/package-legacy.sh 1 dist
```

`dist/<tag>/` then holds each release's files and a `RELEASE.txt` with its title. Publish them as GitHub releases
(not pre-releases). The archives carry no precompiled exporters (building them takes Mojang's jars):
`horizon-prepare` compiles them from the Java sources in the archive on the first preparation, with a Java 25 JDK
the launcher finds or downloads. `packaging/LEGACY.txt` tells players so.
