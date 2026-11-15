//! Entities: the spawn packets 1.19 and 1.20.2 folded into one, and synced entity data, whose
//! serializers 1.19 to 1.19.4 renumbered and whose fields a few entity types moved.
use super::{
    Clientbound, In, Wire, World, nbt, slot,
    tables::{v758, v764},
};
use crate::{Error, Result, buf::Writer};

/// 1.20.2's EntityDataSerializers the front writes by name.
mod serializer {
    pub const VARINT: i32 = 1;
    pub const CAT_VARIANT: i32 = 21;
    pub const PAINTING_VARIANT: i32 = 24;
}

/// 1.20.2's serializer for each of the old wire's, in the old order: a var long (2) came in after the
/// VarInt, a block state (14) before the optional one, and the variants after the pose.
const SERIALIZERS: [i32; 19] = [
    0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 18, 19, 20,
];

/// Particle options, laid out for 1.20.2. `particle` is the old wire's id; `None` when 1.20.2 has no
/// such particle. 1.17.1's barrier and light became block markers of those blocks.
pub fn particle(wire: &Wire, particle: i32, i: &mut In, w: &mut Writer) -> Result<Option<i32>> {
    let p = &wire.particles;
    if particle == p.barrier || particle == p.light {
        w.varint(if particle == p.barrier {
            v764::BARRIER_STATE
        } else {
            v764::LIGHT_STATE
        });
        return Ok(Some(v764::BLOCK_MARKER_PARTICLE));
    }
    if particle == p.block || particle == p.block_marker || particle == p.falling_dust {
        w.varint(wire.state(i.varint()?));
    } else if particle == p.dust {
        w.bytes(i.bytes(16)?);
    } else if particle == p.dust_color_transition {
        w.bytes(i.bytes(28)?);
    } else if particle == p.item {
        slot::up(wire, i, w)?;
    } else if particle == p.vibration {
        // The origin went in 1.19: the packet's own position is where the vibration starts.
        i.i64()?;
        let source = i.string(32767)?;
        w.string(&source);
        match source.as_str() {
            "minecraft:block" | "block" => w.i64(i.i64()?),
            "minecraft:entity" | "entity" => {
                w.varint(i.varint()?);
                w.f32(0.0); // The height on the entity the vibration aims at.
            }
            _ => return Err(Error::new("unknown vibration destination")),
        }
        w.varint(i.varint()?);
    }
    Ok(wire.particle_type.up(particle))
}

/// One value of the old serializer `kind`, as 1.20.2's serializer reads it.
fn value(wire: &Wire, kind: i32, i: &mut In, w: &mut Writer) -> Result<()> {
    match kind {
        // Byte, boolean.
        0 | 7 => w.u8(i.u8()?),
        // VarInt, direction, optional VarInt, pose.
        1 | 11 | 17 | 18 => w.varint(i.varint()?),
        // Float.
        2 => w.bytes(i.bytes(4)?),
        // String, component.
        3 | 4 => w.string(&i.string(262144)?),
        // Optional component.
        5 => {
            let present = i.bool()?;
            w.bool(present);
            if present {
                w.string(&i.string(262144)?);
            }
        }
        6 => slot::up(wire, i, w)?,
        // Rotations.
        8 => w.bytes(i.bytes(12)?),
        // Block position.
        9 => w.i64(i.i64()?),
        10 => {
            let present = i.bool()?;
            w.bool(present);
            if present {
                w.i64(i.i64()?);
            }
        }
        // Optional UUID.
        12 => {
            let present = i.bool()?;
            w.bool(present);
            if present {
                w.uuid(i.uuid()?);
            }
        }
        // Optional block state: 0 is none (and air).
        13 => w.varint(wire.state(i.varint()?)),
        14 => nbt::to_network(i, w)?,
        15 => {
            let id = i.varint()?;
            let mut options = Writer::new();
            // A particle 1.20.2 lacks cannot be left out of a value: the first particle stands in.
            let particle = particle(wire, id, i, &mut options)?;
            w.varint(particle.unwrap_or(0));
            if particle.is_some() {
                w.bytes(&options.data);
            }
        }
        // Villager data.
        16 => {
            for _ in 0..3 {
                w.varint(i.varint()?);
            }
        }
        _ => return Err(Error::new(format!("unknown entity data serializer {kind}"))),
    }
    Ok(())
}

/// Where a field of an entity of `kind` (1.20.2's type id) went: its new index and serializer, and its
/// value when that changed. `None` for a field 1.20.2 no longer has.
fn moved(kind: Option<i32>, index: u8, serializer: i32, value: &mut Vec<u8>) -> Option<(u8, i32)> {
    use v764::entity as e;
    let Some(kind) = kind else {
        return Some((index, serializer));
    };
    match kind {
        // 1.19.3 took the owner (18) out of AbstractHorse: what follows moved down.
        e::HORSE
        | e::DONKEY
        | e::MULE
        | e::LLAMA
        | e::TRADER_LLAMA
        | e::SKELETON_HORSE
        | e::ZOMBIE_HORSE => match index {
            18 => None,
            19.. => Some((index - 1, serializer)),
            _ => Some((index, serializer)),
        },
        // The cat's type became a variant of the same order.
        e::CAT if index == 19 && serializer == serializer::VARINT => {
            Some((index, serializer::CAT_VARIANT))
        }
        // 1.19.4 put the cherry boat before the dark oak one.
        e::BOAT if index == 11 && serializer == serializer::VARINT => {
            if value.as_slice() == [5] {
                *value = vec![6];
            }
            Some((index, serializer))
        }
        _ => Some((index, serializer)),
    }
}

/// A minecart's displayed block is a block state in a plain VarInt (field 11).
fn is_minecart(kind: Option<i32>) -> bool {
    use v764::entity as e;
    matches!(
        kind,
        Some(
            e::MINECART
                | e::CHEST_MINECART
                | e::COMMAND_BLOCK_MINECART
                | e::FURNACE_MINECART
                | e::HOPPER_MINECART
                | e::SPAWNER_MINECART
                | e::TNT_MINECART
        )
    )
}

/// An entity's synced data.
pub fn data(wire: &Wire, world: &World, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let entity = i.varint()?;
    w.varint(entity);
    let kind = world.kinds.get(&entity).copied();
    loop {
        let index = i.u8()?;
        if index == 0xff {
            break;
        }
        let old = i.varint()?;
        let serializer = *usize::try_from(old)
            .ok()
            .and_then(|old| SERIALIZERS.get(old))
            .ok_or_else(|| Error::new(format!("unknown entity data serializer {old}")))?;
        let mut field = Writer::new();
        if index == 11 && old == 1 && is_minecart(kind) {
            field.varint(wire.state(i.varint()?));
        } else {
            value(wire, old, &mut i, &mut field)?;
        }
        if let Some((index, serializer)) = moved(kind, index, serializer, &mut field.data) {
            w.u8(index);
            w.varint(serializer);
            w.bytes(&field.data);
        }
    }
    i.end()?;
    w.u8(0xff);
    Ok(w.data)
}

/// 1.20.2's spawn packet.
struct Spawn {
    id: i32,
    uuid: u128,
    kind: i32,
    position: [f64; 3],
    pitch: i8,
    yaw: i8,
    head_yaw: i8,
    data: i32,
    velocity: [i16; 3],
}

fn spawn(world: &mut World, s: &Spawn) -> (i32, Vec<u8>) {
    world.kinds.insert(s.id, s.kind);
    let mut w = Writer::new();
    w.varint(s.id);
    w.uuid(s.uuid);
    w.varint(s.kind);
    for v in s.position {
        w.f64(v);
    }
    w.i8(s.pitch);
    w.i8(s.yaw);
    w.i8(s.head_yaw);
    w.varint(s.data);
    for v in s.velocity {
        w.i16(v);
    }
    (v764::cb::SPAWN_ENTITY, w.data)
}

fn kind(wire: &Wire, old: i32) -> Result<i32> {
    wire.entity_type
        .up(old)
        .ok_or_else(|| Error::new(format!("unknown entity type {old}")))
}

/// The four spawn packets of before 1.19 (and the player's, which 1.20.2 folded in), as the one.
pub fn spawned(wire: &Wire, world: &mut World, id: i32, payload: &[u8]) -> Result<Clientbound> {
    let mut i = In::new(payload);
    let entity = i.varint()?;
    let uuid = i.uuid()?;
    let position = |i: &mut In| -> Result<[f64; 3]> { Ok([i.f64()?, i.f64()?, i.f64()?]) };
    let velocity = |i: &mut In| -> Result<[i16; 3]> { Ok([i.i16()?, i.i16()?, i.i16()?]) };
    let mut out = Vec::with_capacity(2);
    match id {
        v758::cb::SPAWN_ENTITY => {
            let kind = kind(wire, i.varint()?)?;
            let position = position(&mut i)?;
            let (pitch, yaw) = (i.i8()?, i.i8()?);
            let data = i.i32()?;
            out.push(spawn(
                world,
                &Spawn {
                    id: entity,
                    uuid,
                    kind,
                    position,
                    pitch,
                    yaw,
                    head_yaw: yaw,
                    // A falling block's data is its block state.
                    data: if kind == v764::entity::FALLING_BLOCK {
                        wire.state(data)
                    } else {
                        data
                    },
                    velocity: velocity(&mut i)?,
                },
            ));
        }
        v758::cb::SPAWN_ENTITY_LIVING => {
            let kind = kind(wire, i.varint()?)?;
            let position = position(&mut i)?;
            let (yaw, pitch, head_yaw) = (i.i8()?, i.i8()?, i.i8()?);
            out.push(spawn(
                world,
                &Spawn {
                    id: entity,
                    uuid,
                    kind,
                    position,
                    pitch,
                    yaw,
                    head_yaw,
                    data: 0,
                    velocity: velocity(&mut i)?,
                },
            ));
        }
        v758::cb::NAMED_ENTITY_SPAWN => {
            let position = position(&mut i)?;
            let (yaw, pitch) = (i.i8()?, i.i8()?);
            out.push(spawn(
                world,
                &Spawn {
                    id: entity,
                    uuid,
                    kind: v764::entity::PLAYER,
                    position,
                    pitch,
                    yaw,
                    head_yaw: yaw,
                    data: 0,
                    velocity: [0; 3],
                },
            ));
        }
        v758::cb::SPAWN_ENTITY_PAINTING => {
            let motive = i.varint()?;
            let [x, y, z] = crate::buf::unpack_block_pos(i.i64()?);
            // The packet's 2D direction (south, west, north, east) as the 3D one the spawn data holds.
            let direction = match i.u8()? {
                0 => 3,
                1 => 4,
                2 => 2,
                _ => 5,
            };
            out.push(spawn(
                world,
                &Spawn {
                    id: entity,
                    uuid,
                    kind: v764::entity::PAINTING,
                    position: [f64::from(x), f64::from(y), f64::from(z)],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: direction,
                    velocity: [0; 3],
                },
            ));
            // The motive is entity data since 1.19.
            let variant = usize::try_from(motive)
                .ok()
                .and_then(|motive| v764::PAINTING_VARIANTS.get(motive))
                .copied()
                .unwrap_or(0);
            let mut w = Writer::new();
            w.varint(entity);
            w.u8(8);
            w.varint(serializer::PAINTING_VARIANT);
            w.varint(i32::from(variant));
            w.u8(0xff);
            out.push((v764::cb::ENTITY_METADATA, w.data));
        }
        _ => return Err(Error::new("not a spawn packet")),
    }
    i.end()?;
    Ok(out)
}

/// Entities removed: the front forgets their types.
pub fn removed(world: &mut World, payload: &[u8]) -> Result<()> {
    let mut i = In::new(payload);
    for _ in 0..i.counted()? {
        let id = i.varint()?;
        world.kinds.remove(&id);
    }
    Ok(())
}

/// An entity's equipment: each stack renumbered.
pub fn equipment(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.varint(i.varint()?);
    loop {
        let slot = i.u8()?;
        w.u8(slot);
        slot::up(wire, &mut i, &mut w)?;
        if slot & 0x80 == 0 {
            break;
        }
    }
    i.end()?;
    Ok(w.data)
}
