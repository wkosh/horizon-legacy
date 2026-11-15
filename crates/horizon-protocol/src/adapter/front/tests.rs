//! The front's rewrites, checked against bytes laid out by hand from the protocol's documentation
//! (wiki.vg's pages for 756, 758 and 764) and block state ids read off each release's block list.
use super::*;
use crate::buf::Writer;

fn named(tag: &Tag) -> Vec<u8> {
    let named = horizon_world::nbt::NamedTag {
        name: Default::default(),
        tag: tag.clone(),
    };
    horizon_world::nbt::write_named(&named, horizon_world::nbt::Limits::default()).unwrap()
}

/// Block states (1.17.1 and 1.18.2 number them alike) and 1.20.2's for the same block.
const WATER: (i32, i32) = (34, 80);
const CHEST: (i32, i32) = (2091, 2955);
const DEEPSLATE: (i32, i32) = (18684, 22591);

#[test]
fn block_states_keep_their_block_across_the_inserted_woods() {
    for wire in [&WIRE_758, &WIRE_756] {
        for (old, new) in [(0, 0), (1, 1), WATER, CHEST, DEEPSLATE] {
            assert_eq!(wire.state(old), new, "{}", wire.version.name());
        }
        // Past the last state: air, never another block.
        assert_eq!(wire.state(20342), 0);
        assert_eq!(wire.state(-1), 0);
    }
}

#[test]
fn a_named_tag_loses_its_root_name_and_back() {
    let tag = Tag::compound([("a", Tag::Int(7))]);
    let old = named(&tag);
    let mut w = Writer::new();
    nbt::to_network(&mut In::new(&old), &mut w).unwrap();
    // Type, then the payload without the u16 name length.
    assert_eq!(w.data, [&[10u8][..], &old[3..]].concat());
    let mut back = Writer::new();
    nbt::to_named(&mut In::new(&w.data), &mut back).unwrap();
    assert_eq!(back.data, old);
    // "No tag" is a lone End either way.
    let mut none = Writer::new();
    nbt::to_network(&mut In::new(&[0]), &mut none).unwrap();
    assert_eq!(none.data, [0]);
}

#[test]
fn a_single_valued_section_is_renumbered() {
    let mut world = World {
        height: 32,
        ..World::default()
    };
    let mut p = Writer::new();
    p.i32(3);
    p.i32(-4);
    p.bytes(&named(&Tag::compound([] as [(&str, Tag); 0])));
    let mut sections = Writer::new();
    for state in [WATER.0, DEEPSLATE.0] {
        sections.i16(4096);
        sections.u8(0);
        sections.varint(state);
        sections.varint(0);
        // Biomes: single valued, the server's id 2.
        sections.bytes(&[0, 2, 0]);
    }
    p.varint(sections.data.len() as i32);
    p.bytes(&sections.data);
    p.varint(0);
    p.bool(true); // Trust edges.
    p.bytes(&[0; 6]);
    let out = chunk::column(&WIRE_758, &mut world, &p.data).unwrap();

    let mut expected = Writer::new();
    expected.i32(3);
    expected.i32(-4);
    expected.bytes(&[10, 0]);
    let mut sections = Writer::new();
    for state in [WATER.1, DEEPSLATE.1] {
        sections.i16(4096);
        sections.u8(0);
        sections.varint(state);
        sections.varint(0);
        sections.bytes(&[0, 2, 0]);
    }
    expected.varint(sections.data.len() as i32);
    expected.bytes(&sections.data);
    expected.varint(0);
    expected.bytes(&[0; 6]);
    assert_eq!(out, expected.data);
}

#[test]
fn a_1_17_column_takes_the_light_sent_before_it_and_splits_its_biomes() {
    let mut world = World {
        height: 32,
        biomes: 3,
        ..World::default()
    };
    // The light comes first, for a column not loaded yet: kept, nothing passed on.
    let mut light = Writer::new();
    light.varint(5);
    light.varint(6);
    light.bool(true);
    light.bytes(&[1, 2, 3]);
    assert_eq!(
        chunk::light(&WIRE_756, &mut world, &light.data).unwrap(),
        None
    );

    let mut p = Writer::new();
    p.i32(5);
    p.i32(6);
    p.long_array(&[0b01]); // Only the lowest section is sent.
    p.bytes(&named(&Tag::compound([] as [(&str, Tag); 0])));
    // 2 sections × 64 biome cells: the first cell 2, the rest 1.
    p.varint(128);
    p.varint(2);
    for _ in 1..128 {
        p.varint(1);
    }
    let mut section = Writer::new();
    section.i16(4096);
    section.u8(4);
    section.varint(1);
    section.varint(WATER.0);
    section.long_array(&[0; 256]);
    p.varint(section.data.len() as i32);
    p.bytes(&section.data);
    p.varint(0);
    let out = chunk::column_1_17(&WIRE_756, &mut world, &p.data).unwrap();

    let mut expected = Writer::new();
    expected.i32(5);
    expected.i32(6);
    expected.bytes(&[10, 0]);
    let mut sections = Writer::new();
    sections.i16(4096);
    sections.u8(4);
    sections.varint(1);
    sections.varint(WATER.1);
    sections.long_array(&[0; 256]);
    // Two biomes: one bit each, palette [2, 1]; cell 0 is entry 0, the others entry 1.
    sections.u8(1);
    sections.varint(2);
    sections.varint(2);
    sections.varint(1);
    sections.long_array(&[-2]);
    // The section not sent is air, its biomes all 1.
    sections.i16(0);
    sections.bytes(&[0, 0, 0]);
    sections.bytes(&[0, 1, 0]);
    expected.varint(sections.data.len() as i32);
    expected.bytes(&sections.data);
    expected.varint(0);
    expected.bytes(&[1, 2, 3]);
    assert_eq!(out, expected.data);

    // The column is there now: its next light update goes straight on, without the flag.
    let passed = chunk::light(&WIRE_756, &mut world, &light.data).unwrap();
    assert_eq!(passed, Some(vec![5, 6, 1, 2, 3]));
}

#[test]
fn horse_data_drops_the_owner_and_moves_the_variant_down() {
    let mut world = World::default();
    world.kinds.insert(9, v764::entity::HORSE);
    let mut p = Writer::new();
    p.varint(9);
    p.u8(17); // Flags: a byte.
    p.varint(0);
    p.u8(4);
    p.u8(18); // Owner: an optional UUID, 1.18.2's serializer 12.
    p.varint(12);
    p.bool(true);
    p.uuid(0xABCD);
    p.u8(19); // Variant: a VarInt.
    p.varint(1);
    p.varint(513);
    p.u8(0xff);
    let out = entity::data(&WIRE_758, &world, &p.data).unwrap();
    assert_eq!(out, [9, 17, 0, 4, 18, 1, 0x81, 0x04, 0xff]);
}

#[test]
fn entity_data_serializers_are_renumbered() {
    let world = World::default();
    // An unknown entity's optional block state (1.18.2's 13): 1.20.2's 14, the state renumbered.
    let mut p = Writer::new();
    p.varint(1);
    p.u8(10);
    p.varint(13);
    p.varint(CHEST.0);
    p.u8(0xff);
    let mut expected = Writer::new();
    expected.varint(1);
    expected.u8(10);
    expected.varint(15);
    expected.varint(CHEST.1);
    expected.u8(0xff);
    assert_eq!(
        entity::data(&WIRE_758, &world, &p.data).unwrap(),
        expected.data
    );
}

#[test]
fn command_parsers_are_numbered_and_old_registry_parsers_become_resources() {
    let mut p = Writer::new();
    p.varint(4);
    // Root, children 1 and 3.
    p.bytes(&[0, 2, 1, 3]);
    // Literal "effect", child 2.
    p.bytes(&[1, 1, 2]);
    p.string("effect");
    // Argument "effect" of minecraft:mob_effect, executable.
    p.bytes(&[2 | 4, 0]);
    p.string("effect");
    p.string("minecraft:mob_effect");
    // Argument "n" of brigadier:integer with a minimum of 1, suggestions asked of the server.
    p.bytes(&[2 | 0x10, 0]);
    p.string("n");
    p.string("brigadier:integer");
    p.u8(1);
    p.i32(1);
    p.string("minecraft:ask_server");
    p.varint(0);
    let out = commands::tree(&p.data).unwrap();

    let mut expected = Writer::new();
    expected.varint(4);
    expected.bytes(&[0, 2, 1, 3]);
    expected.bytes(&[1, 1, 2]);
    expected.string("effect");
    expected.bytes(&[2 | 4, 0]);
    expected.string("effect");
    expected.varint(43); // minecraft:resource
    expected.string("minecraft:mob_effect");
    expected.bytes(&[2 | 0x10, 0]);
    expected.string("n");
    expected.varint(3); // brigadier:integer
    expected.u8(1);
    expected.i32(1);
    expected.string("minecraft:ask_server");
    expected.varint(0);
    assert_eq!(out, expected.data);
}

#[test]
fn a_1_17_effect_is_a_byte_from_one() {
    // Entity 3 gets effect 1 (speed) at amplifier 2 for 600 ticks, flags 4.
    let old = [3, 1, 2, 0xd8, 0x04, 4];
    let out = play::clientbound(
        &WIRE_756,
        &mut World::default(),
        v758::cb::ENTITY_EFFECT,
        &old,
    )
    .unwrap();
    // 1.20.2: speed is 0, a VarInt, and no factor data.
    assert_eq!(
        out,
        [(v764::cb::ENTITY_EFFECT, vec![3, 0, 2, 0xd8, 0x04, 4, 0])]
    );
}

#[test]
fn the_join_becomes_a_configuration_then_the_play_login() {
    let front = Front::new(ServerVersion::V1_18_2).unwrap();
    let mut success = Writer::new();
    success.uuid(5);
    success.string("Steve");
    let out = front
        .clientbound(State::Login, v764::login_cb::SUCCESS, &success.data)
        .unwrap();
    // The property list 1.19 added, empty.
    assert_eq!(
        out,
        [(v764::login_cb::SUCCESS, [&success.data[..], &[0]].concat())]
    );

    // A packet before the login waits for it.
    let held = front.clientbound(State::Configuration, v758::cb::UPDATE_TIME, &[0; 16]);
    assert!(held.unwrap().is_empty());

    let overworld = Tag::compound([
        ("min_y", Tag::Int(-64)),
        ("height", Tag::Int(384)),
        (
            "infiniburn",
            Tag::String("minecraft:infiniburn_overworld".into()),
        ),
    ]);
    let codec = Tag::compound([
        (
            "minecraft:dimension_type",
            Tag::compound([(
                "value",
                Tag::List(vec![Tag::compound([
                    ("name", Tag::String("minecraft:overworld".into())),
                    ("id", Tag::Int(0)),
                    ("element", overworld.clone()),
                ])]),
            )]),
        ),
        (
            "minecraft:worldgen/biome",
            Tag::compound([(
                "value",
                Tag::List(
                    ["minecraft:plains", "minecraft:desert"]
                        .iter()
                        .enumerate()
                        .map(|(id, name)| {
                            Tag::compound([
                                ("name", Tag::String((*name).into())),
                                ("id", Tag::Int(id as i32)),
                                (
                                    "element",
                                    Tag::compound([
                                        ("precipitation", Tag::String("rain".into())),
                                        ("category", Tag::String("plains".into())),
                                    ]),
                                ),
                            ])
                        })
                        .collect(),
                ),
            )]),
        ),
    ]);
    let mut login = Writer::new();
    login.i32(42);
    login.bool(false);
    login.u8(1);
    login.i8(-1);
    login.varint(1);
    login.string("minecraft:overworld");
    login.bytes(&named(&codec));
    login.bytes(&named(&overworld));
    login.string("minecraft:overworld");
    login.i64(99);
    login.varint(20);
    login.varint(10);
    login.varint(8);
    for flag in [false, true, false, false] {
        login.bool(flag);
    }
    let out = front
        .clientbound(State::Configuration, v758::cb::LOGIN, &login.data)
        .unwrap();
    let ids: Vec<i32> = out.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        [
            v764::config_cb::REGISTRY_DATA,
            v764::config_cb::FEATURE_FLAGS,
            v764::config_cb::FINISH_CONFIGURATION,
            v764::cb::LOGIN,
            v764::cb::UPDATE_TIME,
        ]
    );
    let registries = crate::buf::Reader::new(&out[0].1).nbt().unwrap();
    let biome = registries
        .get("minecraft:worldgen/biome")
        .and_then(|r| r.get("value"))
        .and_then(|v| match v {
            Tag::List(list) => list.get(1).cloned(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        biome.get("name"),
        Some(&Tag::String("minecraft:desert".into()))
    );
    let element = biome.get("element").unwrap();
    assert_eq!(element.get("has_precipitation"), Some(&Tag::Byte(1)));
    assert_eq!(element.get("category"), None);

    let mut expected = Writer::new();
    expected.i32(42);
    expected.bool(false);
    expected.varint(1);
    expected.string("minecraft:overworld");
    expected.varint(20);
    expected.varint(10);
    expected.varint(8);
    expected.bool(false);
    expected.bool(true);
    expected.bool(false);
    expected.string("minecraft:overworld");
    expected.string("minecraft:overworld");
    expected.i64(99);
    expected.u8(1);
    expected.i8(-1);
    expected.bool(false);
    expected.bool(false);
    expected.bool(false);
    expected.varint(0);
    assert_eq!(out[3].1, expected.data);
    let world = front.world.lock().unwrap();
    assert_eq!((world.min_y, world.height, world.biomes), (-64, 384, 2));
}

#[test]
fn a_block_dig_is_sent_without_its_sequence_and_acknowledged_at_once() {
    let mut p = Writer::new();
    p.varint(0);
    p.i64(0x0123_4567_89ab_cdef);
    p.i8(1);
    p.varint(300);
    let sent =
        serverbound::translate(&WIRE_758, State::Play, v764::sb::BLOCK_DIG, &p.data).unwrap();
    let mut wire = vec![v758::sb::BLOCK_DIG as u8];
    wire.extend_from_slice(&p.data[..1 + 8 + 1]);
    assert_eq!(sent.wire, [wire]);
    assert_eq!(
        sent.incoming,
        [(v764::cb::ACKNOWLEDGE_PLAYER_DIGGING, vec![0xac, 0x02])]
    );
}

#[test]
fn chat_and_commands_are_the_one_old_chat_packet() {
    let mut command = Writer::new();
    command.string("time set day");
    command.i64(1);
    command.i64(2);
    command.varint(0);
    command.varint(0);
    command.bytes(&[0; 3]);
    let sent = serverbound::translate(
        &WIRE_756,
        State::Play,
        v764::sb::CHAT_COMMAND,
        &command.data,
    )
    .unwrap();
    let mut expected = Writer::new();
    expected.varint(v758::sb::CHAT);
    expected.string("/time set day");
    assert_eq!(sent.wire, [expected.data]);
}

#[test]
fn settings_sent_while_configuring_go_as_the_play_packet() {
    let mut settings = Writer::new();
    settings.string("pt_br");
    settings.i8(12);
    settings.varint(0);
    settings.bool(true);
    settings.u8(0x7f);
    settings.varint(1);
    settings.bool(false);
    settings.bool(true);
    let on_758 = serverbound::translate(
        &WIRE_758,
        State::Configuration,
        v764::config_sb::SETTINGS,
        &settings.data,
    )
    .unwrap();
    assert_eq!(on_758.wire, [[&[5u8][..], &settings.data].concat()]);
    // 1.17.1 has no "allow server listings".
    let on_756 = serverbound::translate(
        &WIRE_756,
        State::Configuration,
        v764::config_sb::SETTINGS,
        &settings.data,
    )
    .unwrap();
    let short = &settings.data[..settings.data.len() - 1];
    assert_eq!(on_756.wire, [[&[5u8][..], short].concat()]);
}
