# Joining 1.18.2 and 1.17.1 servers

This fork (`wkosh/horizon-legacy`) extends Horizon's native protocol adapter down to Minecraft 1.18.2
(protocol 758) and 1.17.1 (protocol 756). Everything is translated in Rust inside the connection: no proxy,
no ViaVersion, no Java. Like the rest of the adapter it is off by default:

```sh
HORIZON_OLDER_SERVERS=1 horizon-client ...
```

With it on, the server list marks 1.18.2 and 1.17.1 servers as joinable and the connection picks the
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
