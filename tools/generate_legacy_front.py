#!/usr/bin/env python3
"""Generate crates/horizon-protocol/src/adapter/front/tables.rs: what the legacy front needs to turn the
1.18.2 (758) and 1.17.1 (756) wire into the 1.20.2 (764) one the adapter chain already reads.

    generate_legacy_front.py --data ~/minecraft-data/data

`--data` is the `data/` folder of a checkout of PrismarineJS/minecraft-data (MIT), or a folder holding
`dataPaths.json` and `pc/<version>/{protocol,blocks,items,entities,particles,sounds}.json` copied from it.
Nothing of Mojang's is read: the tables are names and numbers of the public protocol. The generated file
holds, for each old version, its packet ids by name, how they map to 764's, and runs `(from, to, length)`
from its registry ids (block states, blocks, items, entity types, particles, sounds, block entity types)
to 764's and back. Entries are matched by name; block states by block name and properties, a property the
newer block gained taking its default value.
"""

import argparse
import json
import math
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates/horizon-protocol/src/adapter/front/tables.rs"
TARGET = "1.20.2"
OLD = [("1.18.2", "v758", 758), ("1.17.1", "v756", 756)]
PER_LINE = 8
NONE = 65535

# `minecraft:block_entity_type` in 1.20.2's registration order (minecraft-data has no such file), and the
# entries newer than 1.18.2: an older version has the rest in the same order.
BLOCK_ENTITY_TYPES = """
    furnace chest trapped_chest ender_chest jukebox dispenser dropper sign hanging_sign mob_spawner piston
    brewing_stand enchanting_table end_portal beacon skull daylight_detector hopper comparator banner
    structure_block end_gateway command_block shulker_box bed conduit barrel smoker blast_furnace lectern bell
    jigsaw campfire beehive sculk_sensor calibrated_sculk_sensor sculk_catalyst sculk_shrieker
    chiseled_bookshelf brushable_block decorated_pot
""".split()
BLOCK_ENTITY_TYPES_AFTER_1_18 = {
    "hanging_sign", "calibrated_sculk_sensor", "sculk_catalyst", "sculk_shrieker", "chiseled_bookshelf",
    "brushable_block", "decorated_pot",
}
# The `action` byte 1.17.1's block entity data packet carries instead of a registry id.
BLOCK_ENTITY_ACTIONS_756 = {
    1: "mob_spawner", 2: "command_block", 3: "beacon", 4: "skull", 5: "conduit", 6: "banner",
    7: "structure_block", 8: "end_gateway", 9: "sign", 11: "bed", 12: "jigsaw", 13: "campfire", 14: "beehive",
}
# `Motive` ids of the pre-1.19 painting spawn packet, and 1.20.2's `minecraft:painting_variant` (1.19 put four
# paintings before the last).
MOTIVES = """
    kebab aztec alban aztec2 bomb plant wasteland pool courbet sea sunset creebet wanderer graham match bust
    stage void skull_and_roses wither fighters pointer pigscene burning_skull skeleton donkey_kong
""".split()
PAINTING_VARIANTS = MOTIVES[:-1] + ["earth", "wind", "water", "fire", "donkey_kong"]
# Particles whose options the front lays out again, by name.
PARTICLE_CONSTS = ["block", "block_marker", "falling_dust", "dust", "dust_color_transition", "item", "vibration"]
# `minecraft:damage_type` and `minecraft:chat_type` as a 1.20.2 server synchronizes them; an older server has
# neither, so the front names them and the client reads its own data for each.
DAMAGE_TYPES = """
    arrow bad_respawn_point cactus cramming dragon_breath drown dry_out explosion fall falling_anvil
    falling_block falling_stalactite fireball fireworks fly_into_wall freeze generic generic_kill hot_floor
    in_fire in_wall indirect_magic lava lightning_bolt magic mob_attack mob_attack_no_aggro mob_projectile
    on_fire out_of_world outside_border player_attack player_explosion sonic_boom stalagmite starve sting
    sweet_berry_bush thorns thrown trident unattributed_fireball wither wither_skull
""".split()
CHAT_TYPES = """
    chat say_command msg_command_incoming msg_command_outgoing team_msg_command_incoming
    team_msg_command_outgoing emote_command
""".split()
# Entity types whose synced data the front lays out differently, by 764 name.
ENTITY_CONSTS = """
    player painting cat boat horse donkey mule llama trader_llama skeleton_horse zombie_horse enderman
    falling_block item_frame glow_item_frame minecart chest_minecart command_block_minecart furnace_minecart
    hopper_minecart spawner_minecart tnt_minecart area_effect_cloud experience_orb
""".split()


def load(data, version, kind):
    paths = json.loads((data / "dataPaths.json").read_text())["pc"][version]
    folder = paths[kind]
    for candidate in (data / folder / f"{kind}.json", data / version / f"{kind}.json"):
        if candidate.exists():
            return json.loads(candidate.read_text())
    raise SystemExit(f"missing {kind}.json of {version} under {data}")


def packets(protocol, state, direction):
    """{id: name} of one state and direction."""
    if state not in protocol:
        return {}
    mappings = protocol[state][direction]["types"]["packet"][1][0]["type"][1]["mappings"]
    out = {int(k, 16): v for k, v in mappings.items()}
    assert sorted(out) == list(range(len(out))), (state, direction)
    return out


def names(entries):
    """Registry names in id order, ids made to start at 0 where minecraft-data starts them at 1 with no
    entry for 0 (items before 1.20 leave air out; 1.20.2's sounds are listed by holder id)."""
    by_id = {e["id"]: e["name"] for e in entries}
    return by_id


def item_names(entries):
    by_id = names(entries)
    if 0 not in by_id:
        by_id[0] = "air"
    return [by_id[i] for i in range(len(by_id))]


def dense(entries, base=None):
    by_id = names(entries)
    base = min(by_id) if base is None else base
    return [by_id[i + base] for i in range(len(by_id))]


def states(blocks):
    """[(block name, {property: value})] in state id order, and each block's default state id."""
    out = {}
    defaults = {}
    for block in blocks:
        props = block.get("states", [])
        values = []
        for p in props:
            if p["type"] == "bool":
                values.append(["true", "false"])
            elif "values" in p:
                values.append([str(v) for v in p["values"]])
            else:
                values.append([str(v) for v in range(p["num_values"])])
        count = math.prod(len(v) for v in values)
        assert count == block["maxStateId"] - block["minStateId"] + 1, block["name"]
        for index in range(count):
            rest = index
            chosen = {}
            for p, v in zip(reversed(props), reversed(values)):
                chosen[p["name"]] = v[rest % len(v)]
                rest //= len(v)
            out[block["minStateId"] + index] = (block["name"], chosen)
        defaults[block["name"]] = block["defaultState"]
    total = len(out)
    assert sorted(out) == list(range(total))
    return [out[i] for i in range(total)], defaults


def runs(mapping):
    """Runs (from, to, length) of a list `mapping[from] = to or None`."""
    out = []
    for source, target in enumerate(mapping):
        if target is None:
            continue
        if out and out[-1][0] + out[-1][2] == source and out[-1][1] + out[-1][2] == target:
            out[-1] = (out[-1][0], out[-1][1], out[-1][2] + 1)
        else:
            out.append((source, target, 1))
    return out


def invert(mapping, size):
    """The first old id of each new id (several old states can share a new one)."""
    back = [None] * size
    for source, target in enumerate(mapping):
        if target is not None and back[target] is None:
            back[target] = source
    return back


def by_name(old, new, renames=None):
    renames = renames or {}
    index = {name: i for i, name in enumerate(new)}
    return [index.get(renames.get(name, name)) for name in old]


def rust_runs(name, values, doc):
    lines = [f"    /// {doc}", "    #[rustfmt::skip]", f"    pub const {name}: &[(u16, u16, u16)] = &["]
    cells = [f"({a}, {b}, {n})" for a, b, n in values]
    for i in range(0, len(cells), PER_LINE):
        lines.append("        " + ", ".join(cells[i:i + PER_LINE]) + ",")
    lines.append("    ];")
    return lines


def rust_remap(name, mapping, size, doc):
    up = runs(mapping)
    down = runs(invert(mapping, size))
    lines = rust_runs(f"{name}_UP", up, f"{doc}: this version's ids to 764's.")
    lines += rust_runs(f"{name}_DOWN", down, f"{doc}: 764's ids to this version's.")
    lines += [
        f"    pub const {name}: Remap = Remap::Runs {{",
        f"        up: {name}_UP,",
        f"        down: {name}_DOWN,",
        "    };",
    ]
    return lines


def rust_ids(module, table, doc):
    lines = [f"    /// {doc}", f"    pub mod {module} {{"]
    for packet_id, name in sorted(table.items()):
        lines.append(f"        pub const {name.upper()}: i32 = {packet_id};")
    lines.append("    }")
    return lines


def rust_table(name, values, doc):
    lines = [f"    /// {doc}", "    #[rustfmt::skip]", f"    pub const {name}: &[Option<u8>] = &["]
    cells = ["None" if v is None else f"Some({v})" for v in values]
    for i in range(0, len(cells), 12):
        lines.append("        " + ", ".join(cells[i:i + 12]) + ",")
    lines.append("    ];")
    return lines


def rust_strings(name, values, doc, indent="    "):
    lines = [f"{indent}/// {doc}", f"{indent}#[rustfmt::skip]", f"{indent}pub const {name}: &[&str] = &["]
    cells = [json.dumps(v) for v in values]
    for i in range(0, len(cells), 6):
        lines.append(f"{indent}    " + ", ".join(cells[i:i + 6]) + ",")
    lines.append(f"{indent}];")
    return lines


def state_mapping(old_blocks, new_blocks):
    old_states, _ = states(old_blocks)
    new_states, new_defaults = states(new_blocks)
    index = {}
    for state_id, (name, props) in enumerate(new_states):
        index[(name, tuple(sorted(props.items())))] = state_id
    mapping = []
    missing = set()
    for name, props in old_states:
        if name not in new_defaults:
            missing.add(name)
            mapping.append(None)
            continue
        default = dict(new_states[new_defaults[name]][1])
        wanted = {k: props.get(k, v) for k, v in default.items()}
        target = index.get((name, tuple(sorted(wanted.items()))))
        mapping.append(new_defaults[name] if target is None else target)
    return mapping, len(new_states), sorted(missing), new_states, new_defaults


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path, required=True, help="minecraft-data's data/ folder")
    parser.add_argument("--out", type=Path, default=OUT)
    args = parser.parse_args()
    data = args.data

    new_protocol = load(data, TARGET, "protocol")
    new_blocks = load(data, TARGET, "blocks")
    new_items = item_names(load(data, TARGET, "items"))
    new_entities = dense(load(data, TARGET, "entities"))
    new_particles = dense(load(data, TARGET, "particles"))
    new_sounds = dense(load(data, TARGET, "sounds"))
    new_block_names = dense(new_blocks)

    lines = [
        "//! Packet ids and registry ids of 1.18.2 (758) and 1.17.1 (756) against 1.20.2 (764), for the legacy",
        "//! front (`adapter::front`). Generated by tools/generate_legacy_front.py from PrismarineJS/minecraft-data;",
        "//! do not edit by hand.",
        "use crate::adapter::ids::Remap;",
        "",
        "/// 1.20.2, the version the front speaks to the rest of the adapter.",
        "pub mod v764 {",
    ]
    new_cb = packets(new_protocol, "play", "toClient")
    new_sb = packets(new_protocol, "play", "toServer")
    lines += rust_ids("cb", new_cb, "Clientbound play packets.")
    lines += rust_ids("sb", new_sb, "Serverbound play packets.")
    lines += rust_ids("config_cb", packets(new_protocol, "configuration", "toClient"), "Clientbound configuration packets.")
    lines += rust_ids("config_sb", packets(new_protocol, "configuration", "toServer"), "Serverbound configuration packets.")
    lines += rust_ids("login_cb", packets(new_protocol, "login", "toClient"), "Clientbound login packets.")
    lines += rust_ids("login_sb", packets(new_protocol, "login", "toServer"), "Serverbound login packets.")
    lines += ["    /// Entity types whose synced data changed layout since 1.18.2.", "    pub mod entity {"]
    for name in ENTITY_CONSTS:
        lines.append(f"        pub const {name.upper()}: i32 = {new_entities.index(name)};")
    lines.append("    }")
    node = new_protocol["types"]["command_node"][1][3]["type"][1]["fields"]["2"][1][1]["type"][1]["mappings"]
    parsers = [node[str(i)] for i in range(len(node))]
    lines += rust_strings("COMMAND_PARSERS", parsers, "`minecraft:command_argument_type` in id order.")
    lines += rust_strings(
        "BLOCK_ENTITY_TYPES", [f"minecraft:{n}" for n in BLOCK_ENTITY_TYPES], "`minecraft:block_entity_type` in id order."
    )
    lines += [
        "    /// `minecraft:painting_variant` ids by pre-1.19 motive id.",
        "    pub const PAINTING_VARIANTS: &[u8] = &" + json.dumps([PAINTING_VARIANTS.index(n) for n in MOTIVES]) + ";",
    ]
    lines += rust_strings("DAMAGE_TYPES", [f"minecraft:{n}" for n in DAMAGE_TYPES], "`minecraft:damage_type` names.")
    lines += rust_strings("CHAT_TYPES", [f"minecraft:{n}" for n in CHAT_TYPES], "`minecraft:chat_type` names.")
    _, _, _, new_states, new_defaults = state_mapping(new_blocks, new_blocks)
    lines += [
        "    /// Default block states the front writes itself.",
        f"    pub const BARRIER_STATE: i32 = {new_defaults['barrier']};",
        f"    pub const LIGHT_STATE: i32 = {new_defaults['light']};",
        f"    pub const BLOCK_STATES: u32 = {len(new_states)};",
        f"    pub const BLOCK_MARKER_PARTICLE: i32 = {new_particles.index('block_marker')};",
        "}",
    ]

    for version, module, protocol_number in OLD:
        protocol = load(data, version, "protocol")
        cb = packets(protocol, "play", "toClient")
        sb = packets(protocol, "play", "toServer")
        new_cb_ids = {name: i for i, name in new_cb.items()}
        old_sb_ids = {name: i for i, name in sb.items()}
        lines += ["", f"/// {version} (protocol {protocol_number}).", f"pub mod {module} {{", "    use super::Remap;", ""]
        lines += [f"    pub const PROTOCOL: i32 = {protocol_number};", f'    pub const NAME: &str = "{version}";']
        lines += rust_ids("cb", cb, "Clientbound play packets.")
        lines += rust_ids("sb", sb, "Serverbound play packets.")
        lines += rust_table(
            "CLIENTBOUND", [new_cb_ids.get(cb[i]) for i in range(len(cb))],
            "For each clientbound play id, 764's id of the packet with that name.",
        )
        if version != OLD[0][0]:
            first = packets(load(data, OLD[0][0], "protocol"), "play", "toClient")
            first_ids = {name: i for i, name in first.items()}
            assert sb == packets(load(data, OLD[0][0], "protocol"), "play", "toServer")
            lines += [
                f"    /// For each clientbound play id, {OLD[0][0]}'s id of the packet with that name.",
                "    pub const CLIENTBOUND_AS_758: &[u8] = &" + json.dumps([first_ids[cb[i]] for i in range(len(cb))]) + ";",
            ]
        lines += rust_table(
            "SERVERBOUND", [old_sb_ids.get(new_sb[i]) for i in range(len(new_sb))],
            "For each of 764's serverbound play ids, this version's id of the packet with that name.",
        )
        mapping, size, missing, _, _ = state_mapping(load(data, version, "blocks"), new_blocks)
        assert not missing, missing
        lines.append(f"    pub const BLOCK_STATES: u32 = {len(mapping)};")
        lines += rust_remap("BLOCK_STATE", mapping, size, "Block states")
        registries = [
            ("BLOCK", dense(load(data, version, "blocks")), new_block_names, "`minecraft:block`"),
            ("ITEM", item_names(load(data, version, "items")), new_items, "`minecraft:item`"),
            ("ENTITY_TYPE", dense(load(data, version, "entities")), new_entities, "`minecraft:entity_type`"),
            ("PARTICLE_TYPE", dense(load(data, version, "particles")), new_particles, "`minecraft:particle_type`"),
            ("SOUND_EVENT", dense(load(data, version, "sounds")), new_sounds, "`minecraft:sound_event`"),
            (
                "BLOCK_ENTITY_TYPE",
                [n for n in BLOCK_ENTITY_TYPES if n not in BLOCK_ENTITY_TYPES_AFTER_1_18],
                BLOCK_ENTITY_TYPES,
                "`minecraft:block_entity_type`",
            ),
        ]
        for name, old, new, doc in registries:
            mapping = by_name(old, new)
            gone = [n for n, m in zip(old, mapping) if m is None]
            lines += rust_remap(name, mapping, len(new), doc)
            if name == "SOUND_EVENT":
                lines += [
                    "    /// Sounds 1.20.2 has no id for, by this version's id: played by name.",
                    "    pub const SOUNDS_BY_NAME: &[(u16, &str)] = &[",
                ]
                lines += [f'        ({old.index(n)}, "minecraft:{n}"),' for n in gone]
                lines.append("    ];")
            elif name == "PARTICLE_TYPE":
                lines.append("    /// Particles 1.20.2 folded into `block_marker`: this version's ids, -1 where it has none.")
                for particle in ("barrier", "light"):
                    value = old.index(particle) if particle in old else -1
                    lines.append(f"    pub const {particle.upper()}_PARTICLE: i32 = {value};")
                lines += ["    /// Particles with options, -1 where this version has none.", "    pub mod particle {"]
                for particle in PARTICLE_CONSTS:
                    value = old.index(particle) if particle in old else -1
                    lines.append(f"        pub const {particle.upper()}: i32 = {value};")
                lines.append("    }")
                gone = [n for n in gone if n not in ("barrier", "light")]
                assert not gone, (name, version, gone)
            else:
                assert not gone, (name, version, gone)
        if version == "1.17.1":
            lines += [
                "    /// The block entity type (764's id) behind each `action` of this version's block entity data.",
                "    pub const BLOCK_ENTITY_ACTIONS: &[(u8, u8)] = &[",
            ]
            lines += [
                f"        ({action}, {BLOCK_ENTITY_TYPES.index(name)}),"
                for action, name in sorted(BLOCK_ENTITY_ACTIONS_756.items())
            ]
            lines.append("    ];")
        lines.append("}")

    text = "\n".join(lines) + "\n"
    assert not re.search(r"[^\x00-\x7f]", text)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(text)
    # The few lines `#[rustfmt::skip]` does not cover, as `cargo fmt --check` wants them.
    subprocess.run(["rustfmt", "--edition", "2024", str(args.out)], check=True)
    print(f"wrote {args.out} ({len(args.out.read_text().splitlines())} lines)")


if __name__ == "__main__":
    main()
