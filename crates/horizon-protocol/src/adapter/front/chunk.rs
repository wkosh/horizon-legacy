//! Chunk columns, light and block entities.
//!
//! 1.18.2 lays a column out as 1.20.2 does but for its named NBT, the `trust edges` flag 1.20 dropped
//! and its block state ids. 1.17.1's is of the kind before: a mask of the sections sent, one biome
//! array for the column, block entities as whole tags, and the light in a packet of its own that comes
//! just before the column. The front keeps that light until its column arrives and builds 1.20.2's
//! column of the three.
use super::{In, Wire, World, nbt, tables::v764};
use crate::{Error, Result, buf::Writer, registry::ceil_log2};
use horizon_world::nbt::Tag;

/// Entries of a section's block container.
const BLOCKS: usize = 4096;
/// Entries of a section's biome container (4x4x4).
const BIOMES: usize = 64;
/// 1.20.2's LightData with nothing in it: four empty masks and two empty lists.
const NO_LIGHT: [u8; 6] = [0; 6];
/// Light kept for columns that never arrive is dropped past this many.
const LIGHT_KEPT: usize = 4096;

fn longs<'a>(i: &mut In<'a>) -> Result<&'a [u8]> {
    let longs = i.remaining() / 8;
    let count = i.count(longs)?;
    i.bytes(count * 8)
}

/// One section's block states, renumbered. An indirect palette keeps its packed data; a direct one is
/// unpacked and packed again at 1.20.2's width.
fn states(wire: &Wire, i: &mut In, w: &mut Writer) -> Result<()> {
    let bits = i.u8()?;
    // 1.18's single-valued form. 1.17.1 has none: its 0 is read as the narrowest indirect palette.
    if bits == 0 && !wire.v1_17 {
        w.u8(0);
        w.varint(wire.state(i.varint()?));
        longs(i)?;
        w.varint(0);
        return Ok(());
    }
    if bits <= 8 {
        // Under 4 bits both sides read 4, except 1.20.2's 0, which 1.17.1 has not.
        w.u8(if wire.v1_17 { bits.max(4) } else { bits });
        let count = i.counted()?;
        w.varint(count as i32);
        for _ in 0..count {
            w.varint(wire.state(i.varint()?));
        }
        let data = longs(i)?;
        w.varint((data.len() / 8) as i32);
        w.bytes(data);
        return Ok(());
    }
    // The client reads a direct palette at the width of its own state count, whatever `bits` says.
    let old_bits = usize::from(ceil_log2(wire.block_states as usize));
    let new_bits = usize::from(ceil_log2(v764::BLOCK_STATES as usize));
    let data = longs(i)?;
    let per_old = 64 / old_bits;
    if data.len() / 8 * per_old < BLOCKS {
        return Err(Error::new("short direct block palette"));
    }
    let per_new = 64 / new_bits;
    w.u8(new_bits as u8);
    w.varint(BLOCKS.div_ceil(per_new) as i32);
    let old_mask = (1u64 << old_bits) - 1;
    let mut packed = 0u64;
    let mut filled = 0;
    for (n, long) in data.chunks_exact(8).enumerate() {
        let long = u64::from_be_bytes(long.try_into().expect("8 bytes"));
        for slot in 0..per_old {
            if n * per_old + slot >= BLOCKS {
                break;
            }
            let state = wire.state(((long >> (slot * old_bits)) & old_mask) as i32) as u64;
            packed |= state << (filled * new_bits);
            filled += 1;
            if filled == per_new {
                w.i64(packed as i64);
                packed = 0;
                filled = 0;
            }
        }
    }
    if filled != 0 {
        w.i64(packed as i64);
    }
    Ok(())
}

/// One section's biomes (1.18.2), copied: they are numbered by the server's own registry.
fn biomes(i: &mut In, w: &mut Writer) -> Result<()> {
    let start = i.position();
    let bits = i.u8()?;
    if bits == 0 {
        i.varint()?;
    } else if bits <= 3 {
        for _ in 0..i.counted()? {
            i.varint()?;
        }
    }
    longs(i)?;
    w.bytes(&i.data[start..i.position()]);
    Ok(())
}

/// 1.20.2's biome container for one section of 1.17.1's column array (`ids`, 64 cells).
fn biome_section(ids: &[i32], registry: usize, w: &mut Writer) {
    let first = ids.first().copied().unwrap_or(0);
    if ids.len() != BIOMES || ids.iter().all(|&id| id == first) {
        w.u8(0);
        w.varint(first);
        w.varint(0);
        return;
    }
    let mut palette: Vec<i32> = Vec::with_capacity(8);
    for &id in ids {
        if !palette.contains(&id) {
            palette.push(id);
        }
    }
    let bits = if palette.len() <= 8 {
        usize::from(ceil_log2(palette.len()).max(1))
    } else {
        // Direct: the width of the registry, which the client takes from its own count.
        usize::from(ceil_log2(registry.max(2)))
    };
    w.u8(bits as u8);
    if palette.len() <= 8 {
        w.varint(palette.len() as i32);
        for &id in &palette {
            w.varint(id);
        }
    }
    let per_long = 64 / bits;
    w.varint(BIOMES.div_ceil(per_long) as i32);
    for cells in ids.chunks(per_long) {
        let mut packed = 0u64;
        for (slot, &id) in cells.iter().enumerate() {
            let value = if palette.len() <= 8 {
                palette.iter().position(|&p| p == id).unwrap_or(0) as u64
            } else {
                id.max(0) as u64 & ((1 << bits) - 1)
            };
            packed |= value << (slot * bits);
        }
        w.i64(packed as i64);
    }
}

fn block_entity_type(name: &str) -> Option<i32> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    v764::BLOCK_ENTITY_TYPES
        .iter()
        .position(|known| known.strip_prefix("minecraft:") == Some(name))
        .map(|id| id as i32)
}

fn tag_int(tag: &Tag, key: &str) -> Option<i32> {
    match tag.get(key) {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    }
}

fn tag_type(tag: &Tag) -> Option<i32> {
    match tag.get("id") {
        Some(Tag::String(id)) => block_entity_type(&id.to_string_lossy()),
        _ => None,
    }
}

/// 1.18.2's column.
pub fn column(wire: &Wire, world: &mut World, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.data.reserve(payload.len() + 64);
    w.i32(i.i32()?);
    w.i32(i.i32()?);
    nbt::to_network(&mut i, &mut w)?;
    let size = i.counted()?;
    let mut sections = In::new(i.bytes(size)?);
    let mut data = Writer::new();
    data.data.reserve(size + 64);
    for _ in 0..(world.height / 16).max(0) {
        if sections.remaining() == 0 {
            break;
        }
        data.i16(sections.i16()?);
        states(wire, &mut sections, &mut data)?;
        biomes(&mut sections, &mut data)?;
    }
    data.bytes(sections.rest());
    w.varint(data.data.len() as i32);
    w.bytes(&data.data);
    let count = i.counted()?;
    let mut entities = Writer::new();
    let mut kept = 0;
    for _ in 0..count {
        let xz = i.u8()?;
        let y = i.i16()?;
        let kind = wire.block_entity_type.up(i.varint()?);
        let mut tag = Writer::new();
        nbt::to_network(&mut i, &mut tag)?;
        if let Some(kind) = kind {
            entities.u8(xz);
            entities.i16(y);
            entities.varint(kind);
            entities.bytes(&tag.data);
            kept += 1;
        }
    }
    w.varint(kept);
    w.bytes(&entities.data);
    i.bool()?; // Trust edges.
    w.bytes(i.rest());
    Ok(w.data)
}

/// 1.17.1's column, with the light that came before it.
pub fn column_1_17(wire: &Wire, world: &mut World, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.data.reserve(payload.len() + 4096);
    let (x, z) = (i.i32()?, i.i32()?);
    w.i32(x);
    w.i32(z);
    let mask = i.long_array()?;
    let sent = |section: usize| {
        mask.get(section / 64)
            .is_some_and(|long| (long >> (section % 64)) & 1 != 0)
    };
    nbt::to_network(&mut i, &mut w)?;
    let count = i.counted()?;
    let mut biomes = Vec::with_capacity(count);
    for _ in 0..count {
        biomes.push(i.varint()?);
    }
    let size = i.counted()?;
    let mut sections = In::new(i.bytes(size)?);
    let mut data = Writer::new();
    data.data.reserve(size + 256);
    for section in 0..(world.height / 16).max(0) as usize {
        if sent(section) {
            data.i16(sections.i16()?);
            states(wire, &mut sections, &mut data)?;
        } else {
            // An all-air section.
            data.i16(0);
            data.u8(0);
            data.varint(0);
            data.varint(0);
        }
        let cells = biomes
            .get(section * BIOMES..(section + 1) * BIOMES)
            .unwrap_or(&[]);
        biome_section(cells, world.biomes, &mut data);
    }
    w.varint(data.data.len() as i32);
    w.bytes(&data.data);
    let count = i.counted()?;
    let mut entities = Writer::new();
    let mut kept = 0;
    for _ in 0..count {
        let tag = nbt::read(&mut i)?;
        let (Some(kind), Some(bx), Some(by), Some(bz)) = (
            tag_type(&tag),
            tag_int(&tag, "x"),
            tag_int(&tag, "y"),
            tag_int(&tag, "z"),
        ) else {
            continue;
        };
        entities.u8((((bx & 15) << 4) | (bz & 15)) as u8);
        entities.i16(by as i16);
        entities.varint(kind);
        entities.nbt(&tag);
        kept += 1;
    }
    i.end()?;
    w.varint(kept);
    w.bytes(&entities.data);
    match world.light.remove(&(x, z)) {
        Some(light) => w.bytes(&light),
        None => w.bytes(&NO_LIGHT),
    }
    world.columns.insert((x, z));
    Ok(w.data)
}

/// A light update without its `trust edges` flag. 1.17.1's for a column that has not arrived is kept
/// for that column instead (`None`).
pub fn light(wire: &Wire, world: &mut World, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut i = In::new(payload);
    let (x, z) = (i.varint()?, i.varint()?);
    i.bool()?;
    if wire.v1_17 && !world.columns.contains(&(x, z)) {
        if world.light.len() >= LIGHT_KEPT {
            world.light.clear();
        }
        world.light.insert((x, z), i.rest().to_vec());
        return Ok(None);
    }
    let mut w = Writer::new();
    w.varint(x);
    w.varint(z);
    w.bytes(i.rest());
    Ok(Some(w.data))
}

/// 1.20.2 writes the column's position as one long: z first.
pub fn unload(world: &mut World, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let (x, z) = (i.i32()?, i.i32()?);
    i.end()?;
    world.columns.remove(&(x, z));
    world.light.remove(&(x, z));
    let mut w = Writer::new();
    w.i32(z);
    w.i32(x);
    Ok(w.data)
}

/// A block entity's data. 1.17.1 gives an `action` for a type: the tag's own id says which, the action
/// where it has none. `None` for a type 1.20.2 does not know.
pub fn block_entity(wire: &Wire, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.i64(i.i64()?);
    if wire.v1_17 {
        let action = i.u8()?;
        let tag = nbt::read(&mut i)?;
        i.end()?;
        let by_action = || {
            super::tables::v756::BLOCK_ENTITY_ACTIONS
                .iter()
                .find(|&&(known, _)| known == action)
                .map(|&(_, kind)| i32::from(kind))
        };
        let Some(kind) = tag_type(&tag).or_else(by_action) else {
            return Ok(None);
        };
        w.varint(kind);
        w.nbt(&tag);
        return Ok(Some(w.data));
    }
    let Some(kind) = wire.block_entity_type.up(i.varint()?) else {
        return Ok(None);
    };
    w.varint(kind);
    nbt::to_network(&mut i, &mut w)?;
    i.end()?;
    Ok(Some(w.data))
}

/// A section's block changes: no `trust edges` flag, and each state renumbered.
pub fn section_blocks(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.i64(i.i64()?);
    i.bool()?;
    let count = i.counted()?;
    w.varint(count as i32);
    for _ in 0..count {
        let change = i.varlong()?;
        let state = i64::from(wire.state((change >> 12) as i32));
        w.varlong((state << 12) | (change & 0xfff));
    }
    i.end()?;
    Ok(w.data)
}
