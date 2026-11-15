//! Item stacks: a flag, the item's id, a count and its tag. The tag is that of the server's data
//! version, which the steps above the front upgrade; the front renumbers the item and unnames the tag.
use super::{In, Wire, nbt};
use crate::{Result, buf::Writer};

/// A stack off the old wire, as 1.20.2 sends it. An item 1.20.2 has no id for becomes an empty slot.
pub fn up(wire: &Wire, i: &mut In, w: &mut Writer) -> Result<()> {
    if !i.bool()? {
        w.bool(false);
        return Ok(());
    }
    let item = wire.item.up(i.varint()?);
    let count = i.i8()?;
    match item {
        Some(item) => {
            w.bool(true);
            w.varint(item);
            w.i8(count);
            nbt::to_network(i, w)
        }
        None => {
            w.bool(false);
            nbt::to_network(i, &mut Writer::new())
        }
    }
}

/// A stack Horizon sends, laid out for 1.20.2, as the old server reads it. An item the server has no
/// id for becomes an empty slot.
pub fn down(wire: &Wire, i: &mut In, w: &mut Writer) -> Result<()> {
    if !i.bool()? {
        w.bool(false);
        return Ok(());
    }
    let item = wire.item.down(i.varint()?);
    let count = i.i8()?;
    match item {
        Some(item) => {
            w.bool(true);
            w.varint(item);
            w.i8(count);
            nbt::to_named(i, w)
        }
        None => {
            w.bool(false);
            nbt::to_named(i, &mut Writer::new())
        }
    }
}
