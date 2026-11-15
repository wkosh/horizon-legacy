//! Clientbound play packets of 1.18.2 (1.17.1's renumbered to it), as 1.20.2 sends them. A packet both
//! lay out alike only changes id; the rest are rewritten here or in the module of what they carry.
use super::{
    Clientbound, In, Wire, World, chunk, commands, entity, join, nbt, slot,
    tables::{v758::cb, v764},
};
use crate::{Error, Result, buf::Writer};

/// The packet 1.20.2 has under the same name, payload untouched.
fn renumbered(wire: &Wire, id: i32, payload: Vec<u8>) -> Clientbound {
    let mapped = usize::try_from(id)
        .ok()
        .and_then(|id| wire.clientbound.get(id).copied().flatten());
    match mapped {
        Some(id) => vec![(i32::from(id), payload)],
        None => Vec::new(),
    }
}

/// One play packet off the wire, in 1.18.2's numbering.
pub fn clientbound(wire: &Wire, world: &mut World, id: i32, payload: &[u8]) -> Result<Clientbound> {
    let one = |id: i32, payload: Vec<u8>| Ok(vec![(id, payload)]);
    let same = |payload: Vec<u8>| Ok(renumbered(wire, id, payload));
    match id {
        cb::LOGIN => one(v764::cb::LOGIN, join::login(wire, world, payload)?),
        cb::RESPAWN => same(join::respawn(world, payload)?),

        cb::MAP_CHUNK if wire.v1_17 => same(chunk::column_1_17(wire, world, payload)?),
        cb::MAP_CHUNK => same(chunk::column(wire, world, payload)?),
        cb::UPDATE_LIGHT => Ok(chunk::light(wire, world, payload)?
            .map_or_else(Vec::new, |payload| renumbered(wire, id, payload))),
        cb::UNLOAD_CHUNK => same(chunk::unload(world, payload)?),
        cb::TILE_ENTITY_DATA => Ok(chunk::block_entity(wire, payload)?
            .map_or_else(Vec::new, |payload| renumbered(wire, id, payload))),
        cb::MULTI_BLOCK_CHANGE => same(chunk::section_blocks(wire, payload)?),
        cb::BLOCK_CHANGE => same(block_change(wire, payload)?),
        // 1.19 acknowledges by sequence, which the front does when the action is sent
        // (`serverbound`); what is left of the old acknowledgement is the block the server holds.
        cb::ACKNOWLEDGE_PLAYER_DIGGING => one(v764::cb::BLOCK_CHANGE, block_change(wire, payload)?),
        cb::BLOCK_ACTION => same(block_action(wire, payload)?),
        cb::WORLD_EVENT => same(world_event(wire, payload)?),
        cb::EXPLOSION => same(explosion(payload)?),

        cb::SPAWN_ENTITY
        | cb::SPAWN_ENTITY_LIVING
        | cb::SPAWN_ENTITY_PAINTING
        | cb::NAMED_ENTITY_SPAWN => entity::spawned(wire, world, id, payload),
        cb::SPAWN_ENTITY_EXPERIENCE_ORB => {
            let entity = In::new(payload).varint()?;
            world.kinds.insert(entity, v764::entity::EXPERIENCE_ORB);
            same(payload.to_vec())
        }
        cb::ENTITY_METADATA => same(entity::data(wire, world, payload)?),
        cb::ENTITY_DESTROY => {
            entity::removed(world, payload)?;
            same(payload.to_vec())
        }
        cb::ENTITY_EQUIPMENT => same(entity::equipment(wire, payload)?),
        cb::ENTITY_EFFECT => same(effect(wire, payload, true)?),
        cb::REMOVE_ENTITY_EFFECT => same(effect(wire, payload, false)?),
        cb::ANIMATION => animation(wire, id, payload),
        cb::ENTITY_STATUS => entity_status(wire, id, payload),
        // 1.19 draws the vibration as a particle the server sends itself.
        cb::SCULK_VIBRATION_SIGNAL => Ok(Vec::new()),

        cb::WINDOW_ITEMS => same(window_items(wire, payload)?),
        cb::SET_SLOT => same(set_slot(wire, payload)?),
        cb::SET_COOLDOWN => same(cooldown(wire, payload)?),
        cb::TRADE_LIST => same(trades(wire, payload)?),
        cb::DECLARE_RECIPES => same(recipes(wire, payload)?),
        cb::ADVANCEMENTS => same(advancements(wire, payload)?),
        cb::STATISTICS => same(statistics(wire, payload)?),
        cb::TAGS => same(tags(wire, payload)?),
        cb::DECLARE_COMMANDS => same(commands::tree(payload)?),

        cb::CHAT => one(v764::cb::SYSTEM_CHAT, chat(payload)?),
        cb::PLAYER_INFO => player_info(payload),
        cb::POSITION => same(position(payload)?),
        cb::WORLD_PARTICLES => Ok(particles(wire, payload)?
            .map_or_else(Vec::new, |payload| renumbered(wire, id, payload))),
        cb::SOUND_EFFECT => same(sound(wire, payload, Sound::Id, false)?),
        cb::ENTITY_SOUND_EFFECT => same(sound(wire, payload, Sound::Id, true)?),
        cb::NAMED_SOUND_EFFECT => one(
            v764::cb::SOUND_EFFECT,
            sound(wire, payload, Sound::Name, false)?,
        ),
        cb::OPEN_SIGN_ENTITY => {
            let mut payload = payload.to_vec();
            payload.push(1); // The front text: a sign had one side.
            same(payload)
        }
        cb::END_COMBAT_EVENT => {
            // 1.20 dropped the opponent's id.
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.varint(i.varint()?);
            same(w.data)
        }
        cb::DEATH_COMBAT_EVENT => {
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.varint(i.varint()?);
            i.i32()?;
            w.bytes(i.rest());
            same(w.data)
        }
        cb::SCOREBOARD_DISPLAY_OBJECTIVE => {
            // The slot is a VarInt since 1.20.2.
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.varint(i32::from(i.i8()?));
            w.bytes(i.rest());
            same(w.data)
        }
        // 1.20.2 sends the border's lerp time as a VarInt.
        cb::INITIALIZE_WORLD_BORDER => same(border(payload, 8 * 4)?),
        cb::WORLD_BORDER_LERP_SIZE => same(border(payload, 8 * 2)?),
        cb::NBT_QUERY_RESPONSE => {
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.varint(i.varint()?);
            nbt::to_network(&mut i, &mut w)?;
            i.end()?;
            same(w.data)
        }
        _ => same(payload.to_vec()),
    }
}

/// A world border packet: `before` bytes, then the lerp time (a VarLong, now a VarInt) and the rest.
fn border(payload: &[u8], before: usize) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.bytes(i.bytes(before)?);
    let time = i.varlong()?;
    w.varint(i32::try_from(time).unwrap_or(i32::MAX));
    w.bytes(i.rest());
    Ok(w.data)
}

fn block_change(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.i64(i.i64()?);
    w.varint(wire.state(i.varint()?));
    Ok(w.data)
}

fn block_action(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.i64(i.i64()?);
    w.u8(i.u8()?);
    w.u8(i.u8()?);
    let block = i.varint()?;
    i.end()?;
    w.varint(
        wire.block
            .up(block)
            .ok_or_else(|| Error::new(format!("unknown block {block}")))?,
    );
    Ok(w.data)
}

/// A level event: the breaking of a block carries its state, a record its item.
fn world_event(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    const PLAY_RECORD: i32 = 1010;
    const DESTROY_BLOCK: i32 = 2001;
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let event = i.i32()?;
    w.i32(event);
    w.i64(i.i64()?);
    let data = i.i32()?;
    w.i32(match event {
        DESTROY_BLOCK => wire.state(data),
        PLAY_RECORD => wire.item.up(data).unwrap_or(0),
        _ => data,
    });
    w.bool(i.bool()?);
    i.end()?;
    Ok(w.data)
}

/// 1.19.3 made the explosion's centre doubles.
fn explosion(payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    for _ in 0..3 {
        w.f64(f64::from(i.f32()?));
    }
    w.bytes(i.rest());
    Ok(w.data)
}

/// A mob effect given (`given`) or removed. The old id is the effect's number from 1, a byte in 1.17.1;
/// 1.20.2's is its registry id, from 0, and the update ends with an optional factor tag.
fn effect(wire: &Wire, payload: &[u8], given: bool) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.varint(i.varint()?);
    let effect = if wire.v1_17 {
        i32::from(i.u8()?)
    } else {
        i.varint()?
    };
    if effect < 1 {
        return Err(Error::new("no such mob effect"));
    }
    w.varint(effect - 1);
    if given {
        w.i8(i.i8()?);
        w.varint(i.varint()?);
        w.i8(i.i8()?);
        w.bool(false);
    }
    i.end()?;
    Ok(w.data)
}

/// 1.19.4 took "hurt" out of the animations: it is a packet of its own, with the direction it came from.
fn animation(wire: &Wire, id: i32, payload: &[u8]) -> Result<Clientbound> {
    const HURT: u8 = 1;
    let mut i = In::new(payload);
    let entity = i.varint()?;
    if i.u8()? != HURT {
        return Ok(renumbered(wire, id, payload.to_vec()));
    }
    let mut w = Writer::new();
    w.varint(entity);
    w.f32(0.0);
    Ok(vec![(v764::cb::HURT_ANIMATION, w.data)])
}

/// 1.19.4 replaced the "hurt" entity events by a damage event naming its type: the client plays the
/// hurt sound and animation from that.
fn entity_status(wire: &Wire, id: i32, payload: &[u8]) -> Result<Clientbound> {
    let mut i = In::new(payload);
    let entity = i.i32()?;
    let damage = match i.i8()? {
        2 => "minecraft:generic",
        33 => "minecraft:thorns",
        36 => "minecraft:drown",
        37 => "minecraft:on_fire",
        44 => "minecraft:sweet_berry_bush",
        57 => "minecraft:freeze",
        _ => return Ok(renumbered(wire, id, payload.to_vec())),
    };
    // The front synchronized the damage types itself, in this order (`join::registry_data`).
    let damage = v764::DAMAGE_TYPES
        .iter()
        .position(|name| *name == damage)
        .unwrap_or(0);
    let mut w = Writer::new();
    w.varint(entity);
    w.varint(damage as i32);
    w.varint(0); // No causing entity,
    w.varint(0); // no direct one,
    w.bool(false); // and no position.
    Ok(vec![(v764::cb::DAMAGE_EVENT, w.data)])
}

fn window_items(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.u8(i.u8()?);
    w.varint(i.varint()?);
    let count = i.counted()?;
    w.varint(count as i32);
    for _ in 0..=count {
        // The last is the carried stack.
        slot::up(wire, &mut i, &mut w)?;
    }
    i.end()?;
    Ok(w.data)
}

fn set_slot(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.i8(i.i8()?);
    w.varint(i.varint()?);
    w.i16(i.i16()?);
    slot::up(wire, &mut i, &mut w)?;
    i.end()?;
    Ok(w.data)
}

fn cooldown(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let item = i.varint()?;
    w.varint(
        wire.item
            .up(item)
            .ok_or_else(|| Error::new(format!("unknown item {item}")))?,
    );
    w.varint(i.varint()?);
    Ok(w.data)
}

/// A merchant's offers: 1.19 counts them with a VarInt and always writes the second cost.
fn trades(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.varint(i.varint()?);
    let count = i.u8()?;
    w.varint(i32::from(count));
    for _ in 0..count {
        slot::up(wire, &mut i, &mut w)?;
        slot::up(wire, &mut i, &mut w)?;
        if i.bool()? {
            slot::up(wire, &mut i, &mut w)?;
        } else {
            w.bool(false);
        }
        // Out of stock, uses, max uses, xp, special price, price multiplier, demand.
        w.bytes(i.bytes(1 + 4 * 6)?);
    }
    w.bytes(i.rest());
    Ok(w.data)
}

fn ingredient(wire: &Wire, i: &mut In, w: &mut Writer) -> Result<()> {
    let count = i.counted()?;
    w.varint(count as i32);
    for _ in 0..count {
        slot::up(wire, i, w)?;
    }
    Ok(())
}

/// The recipes: 1.19.3 gave each a book category, 1.20 a notification flag to shaped ones and a
/// template to smithing.
fn recipes(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    const CRAFTING_MISC: i32 = 3;
    const COOKING_MISC: i32 = 2;
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let count = i.counted()?;
    w.varint(count as i32);
    for _ in 0..count {
        let kind = i.string(32767)?;
        let kind = kind.strip_prefix("minecraft:").unwrap_or(&kind).to_owned();
        let name = i.string(32767)?;
        let smithing = kind == "smithing";
        w.string(&if smithing {
            "minecraft:smithing_transform".to_owned()
        } else {
            format!("minecraft:{kind}")
        });
        w.string(&name);
        match kind.as_str() {
            "crafting_shapeless" => {
                w.string(&i.string(32767)?);
                w.varint(CRAFTING_MISC);
                ingredient(wire, &mut i, &mut w)?;
                slot::up(wire, &mut i, &mut w)?;
            }
            "crafting_shaped" => {
                let (width, height) = (i.varint()?, i.varint()?);
                w.varint(width);
                w.varint(height);
                w.string(&i.string(32767)?);
                w.varint(CRAFTING_MISC);
                let cells = usize::try_from(width)
                    .ok()
                    .zip(usize::try_from(height).ok())
                    .and_then(|(width, height)| width.checked_mul(height))
                    .filter(|&cells| cells <= i.remaining())
                    .ok_or_else(|| Error::new("invalid shaped recipe size"))?;
                for _ in 0..cells {
                    ingredient(wire, &mut i, &mut w)?;
                }
                slot::up(wire, &mut i, &mut w)?;
                w.bool(true);
            }
            "smelting" | "blasting" | "smoking" | "campfire_cooking" => {
                w.string(&i.string(32767)?);
                w.varint(COOKING_MISC);
                ingredient(wire, &mut i, &mut w)?;
                slot::up(wire, &mut i, &mut w)?;
                w.f32(i.f32()?);
                w.varint(i.varint()?);
            }
            "stonecutting" => {
                w.string(&i.string(32767)?);
                ingredient(wire, &mut i, &mut w)?;
                slot::up(wire, &mut i, &mut w)?;
            }
            _ if smithing => {
                w.varint(0); // No template: the old table took none.
                ingredient(wire, &mut i, &mut w)?;
                ingredient(wire, &mut i, &mut w)?;
                slot::up(wire, &mut i, &mut w)?;
            }
            special if special.starts_with("crafting_special_") => w.varint(CRAFTING_MISC),
            other => return Err(Error::new(format!("unknown recipe type {other}"))),
        }
    }
    i.end()?;
    Ok(w.data)
}

/// The advancements: 1.20.2 leaves the criteria's names to the requirements and adds a telemetry flag.
fn advancements(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.bool(i.bool()?);
    let count = i.counted()?;
    w.varint(count as i32);
    for _ in 0..count {
        w.string(&i.string(32767)?);
        let parent = i.bool()?;
        w.bool(parent);
        if parent {
            w.string(&i.string(32767)?);
        }
        let display = i.bool()?;
        w.bool(display);
        if display {
            w.string(&i.string(262144)?);
            w.string(&i.string(262144)?);
            slot::up(wire, &mut i, &mut w)?;
            w.varint(i.varint()?);
            let flags = i.i32()?;
            w.i32(flags);
            if flags & 1 != 0 {
                w.string(&i.string(32767)?);
            }
            w.f32(i.f32()?);
            w.f32(i.f32()?);
        }
        for _ in 0..i.counted()? {
            i.string(32767)?;
        }
        let requirements = i.counted()?;
        w.varint(requirements as i32);
        for _ in 0..requirements {
            let any = i.counted()?;
            w.varint(any as i32);
            for _ in 0..any {
                w.string(&i.string(32767)?);
            }
        }
        w.bool(false);
    }
    w.bytes(i.rest());
    Ok(w.data)
}

/// The statistics: those counted per block, item or entity type are numbered by it.
fn statistics(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut kept = Writer::new();
    let mut count = 0;
    for _ in 0..i.counted()? {
        let kind = i.varint()?;
        let stat = i.varint()?;
        let value = i.varint()?;
        let stat = match kind {
            0 => wire.block.up(stat),
            1..=5 => wire.item.up(stat),
            6 | 7 => wire.entity_type.up(stat),
            _ => Some(stat),
        };
        if let Some(stat) = stat {
            kept.varint(kind);
            kept.varint(stat);
            kept.varint(value);
            count += 1;
        }
    }
    i.end()?;
    let mut w = Writer::new();
    w.varint(count);
    w.bytes(&kept.data);
    Ok(w.data)
}

/// The tags: entries renumbered by their registry. Game events are left out: 1.19 renamed and
/// renumbered them, and their tags only matter to the server.
fn tags(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut kept = Writer::new();
    let mut count = 0;
    for _ in 0..i.counted()? {
        let registry = i.string(32767)?;
        let name = registry.strip_prefix("minecraft:").unwrap_or(&registry);
        let remap = match name {
            "block" => Some(wire.block),
            "item" => Some(wire.item),
            "entity_type" => Some(wire.entity_type),
            "fluid" => Some(super::Remap::Same),
            _ => None,
        };
        let mut w = Writer::new();
        w.string(&registry);
        let tags = i.counted()?;
        w.varint(tags as i32);
        for _ in 0..tags {
            w.string(&i.string(32767)?);
            let entries = i.counted()?;
            let mut ids = Writer::new();
            let mut known = 0;
            for _ in 0..entries {
                let id = i.varint()?;
                if let Some(id) = remap.and_then(|remap| remap.up(id)) {
                    ids.varint(id);
                    known += 1;
                }
            }
            w.varint(known);
            w.bytes(&ids.data);
        }
        if remap.is_some() {
            kept.bytes(&w.data);
            count += 1;
        }
    }
    i.end()?;
    let mut w = Writer::new();
    w.varint(count);
    w.bytes(&kept.data);
    Ok(w.data)
}

/// The one chat packet of before 1.19, as a system message: position 2 is the action bar.
fn chat(payload: &[u8]) -> Result<Vec<u8>> {
    const GAME_INFO: i8 = 2;
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.string(&i.string(262144)?);
    w.bool(i.i8()? == GAME_INFO);
    Ok(w.data)
}

/// The player list: 1.19.3 split removal off and made the update a set of actions per entry.
fn player_info(payload: &[u8]) -> Result<Clientbound> {
    const ADD: u8 = 1;
    const GAME_MODE: u8 = 1 << 2;
    const LISTED: u8 = 1 << 3;
    const LATENCY: u8 = 1 << 4;
    const DISPLAY_NAME: u8 = 1 << 5;
    let mut i = In::new(payload);
    let action = i.varint()?;
    let count = i.counted()?;
    let mut w = Writer::new();
    let actions = match action {
        0 => ADD | GAME_MODE | LISTED | LATENCY | DISPLAY_NAME,
        1 => GAME_MODE,
        2 => LATENCY,
        3 => DISPLAY_NAME,
        4 => 0,
        _ => return Err(Error::new("unknown player list action")),
    };
    if actions != 0 {
        w.u8(actions);
    }
    w.varint(count as i32);
    let display_name = |i: &mut In, w: &mut Writer| -> Result<()> {
        let present = i.bool()?;
        w.bool(present);
        if present {
            w.string(&i.string(262144)?);
        }
        Ok(())
    };
    for _ in 0..count {
        w.uuid(i.uuid()?);
        match action {
            0 => {
                w.string(&i.string(16)?);
                let properties = i.counted()?;
                w.varint(properties as i32);
                for _ in 0..properties {
                    w.string(&i.string(32767)?);
                    w.string(&i.string(32767)?);
                    let signed = i.bool()?;
                    w.bool(signed);
                    if signed {
                        w.string(&i.string(32767)?);
                    }
                }
                w.varint(i.varint()?);
                w.bool(true); // Every player the old list names is shown in it.
                w.varint(i.varint()?);
                display_name(&mut i, &mut w)?;
            }
            1 | 2 => w.varint(i.varint()?),
            3 => display_name(&mut i, &mut w)?,
            _ => {}
        }
    }
    i.end()?;
    Ok(vec![(
        if actions == 0 {
            v764::cb::PLAYER_REMOVE
        } else {
            v764::cb::PLAYER_INFO
        },
        w.data,
    )])
}

/// 1.19.4 dropped the "dismount vehicle" flag at the end.
fn position(payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    // Position, rotation, relative flags.
    w.bytes(i.bytes(8 * 3 + 4 * 2 + 1)?);
    w.varint(i.varint()?);
    Ok(w.data)
}

/// Particles: the id is a VarInt since 1.19, and the options follow 1.20.2's layout. `None` for a
/// particle 1.20.2 lacks.
fn particles(wire: &Wire, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut i = In::new(payload);
    let particle = i.i32()?;
    // Long distance, position, offsets, speed, count.
    let fixed = i.bytes(1 + 8 * 3 + 4 * 3 + 4 + 4)?;
    let mut options = Writer::new();
    let Some(particle) = entity::particle(wire, particle, &mut i, &mut options)? else {
        return Ok(None);
    };
    i.end()?;
    let mut w = Writer::new();
    w.varint(particle);
    w.bytes(fixed);
    w.bytes(&options.data);
    Ok(Some(w.data))
}

/// How the old packet names its sound.
enum Sound {
    Id,
    Name,
}

/// A sound at a position or on an entity (`on_entity`). 1.19.3's holder: the registry id plus one, or
/// 0 and the sound's name with an optional fixed range; 1.19 ends the packet with a seed.
fn sound(wire: &Wire, payload: &[u8], named: Sound, on_entity: bool) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let by_name = |w: &mut Writer, name: &str| {
        w.varint(0);
        w.string(name);
        w.bool(false);
    };
    match named {
        Sound::Name => by_name(&mut w, &i.string(32767)?),
        Sound::Id => {
            let id = i.varint()?;
            match wire.sound_event.up(id) {
                Some(id) => w.varint(id + 1),
                None => {
                    let name = wire
                        .sounds_by_name
                        .iter()
                        .find(|&&(old, _)| i32::from(old) == id)
                        .map(|&(_, name)| name)
                        .ok_or_else(|| Error::new(format!("unknown sound {id}")))?;
                    by_name(&mut w, name);
                }
            }
        }
    }
    w.varint(i.varint()?);
    if on_entity {
        w.varint(i.varint()?);
    } else {
        w.bytes(i.bytes(12)?);
    }
    w.bytes(i.bytes(8)?);
    i.end()?;
    w.i64(0);
    Ok(w.data)
}
