//! The command tree: before 1.19 an argument names its parser by identifier; 1.20.2 numbers them. The
//! parsers 1.19.3 folded into `resource` name their registry instead, and `time` gained a minimum.
use super::{In, tables::v764};
use crate::{Error, Result, buf::Writer};

const HAS_REDIRECT: u8 = 0x08;
const HAS_SUGGESTIONS: u8 = 0x10;
const LITERAL: u8 = 1;
const ARGUMENT: u8 = 2;

/// 1.20.2's number for the parser 1.20.2 names `name`.
fn parser(name: &str) -> Result<i32> {
    v764::COMMAND_PARSERS
        .iter()
        .position(|known| *known == name)
        .map(|id| id as i32)
        .ok_or_else(|| Error::new(format!("unknown command parser {name}")))
}

/// A number's bounds: a flag for each of the minimum and maximum, then those present, `size` bytes each.
fn bounds(i: &mut In, w: &mut Writer, size: usize) -> Result<()> {
    let flags = i.u8()?;
    w.u8(flags);
    w.bytes(i.bytes(size * (flags & 3).count_ones() as usize)?);
    Ok(())
}

/// An argument's parser and its properties.
fn argument(i: &mut In, w: &mut Writer) -> Result<()> {
    let name = i.string(32767)?;
    // The parsers of one registry's entries are `resource` with that registry since 1.19.3.
    let registry = match name.as_str() {
        "minecraft:mob_effect" => Some("minecraft:mob_effect"),
        "minecraft:item_enchantment" => Some("minecraft:enchantment"),
        "minecraft:entity_summon" => Some("minecraft:entity_type"),
        _ => None,
    };
    if let Some(registry) = registry {
        w.varint(parser("minecraft:resource")?);
        w.string(registry);
        return Ok(());
    }
    let name = match name.as_str() {
        "minecraft:nbt_compound_tag" => "minecraft:nbt",
        // 1.18.2's two take keys: 1.19 renamed them when it added the ones that take entries.
        "minecraft:resource_or_tag" => "minecraft:resource_or_tag_key",
        "minecraft:resource" => "minecraft:resource_key",
        other => other,
    };
    w.varint(parser(name)?);
    match name {
        "brigadier:float" | "brigadier:integer" => bounds(i, w, 4)?,
        "brigadier:double" | "brigadier:long" => bounds(i, w, 8)?,
        "brigadier:string" => w.varint(i.varint()?),
        "minecraft:entity" | "minecraft:score_holder" => w.u8(i.u8()?),
        "minecraft:resource_or_tag_key" | "minecraft:resource_key" => w.string(&i.string(32767)?),
        "minecraft:time" => w.i32(0),
        _ => {}
    }
    Ok(())
}

/// The tree as 1.20.2 lays it out.
pub fn tree(payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    let nodes = i.counted()?;
    w.varint(nodes as i32);
    for _ in 0..nodes {
        let flags = i.u8()?;
        w.u8(flags);
        let children = i.counted()?;
        w.varint(children as i32);
        for _ in 0..children {
            w.varint(i.varint()?);
        }
        if flags & HAS_REDIRECT != 0 {
            w.varint(i.varint()?);
        }
        match flags & 3 {
            LITERAL => w.string(&i.string(32767)?),
            ARGUMENT => {
                w.string(&i.string(32767)?);
                argument(&mut i, &mut w)?;
            }
            _ => {}
        }
        if flags & HAS_SUGGESTIONS != 0 {
            let suggestions = i.string(32767)?;
            // 1.19 took the biome list out: the server is asked, as for any other.
            w.string(if suggestions == "minecraft:available_biomes" {
                "minecraft:ask_server"
            } else {
                &suggestions
            });
        }
    }
    w.varint(i.varint()?);
    i.end()?;
    Ok(w.data)
}
