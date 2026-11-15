//! NBT on the old wire: every tag has a root name (always empty), where 1.20.2's network form has none.
//! A tag's payload is the same bytes either way, so it is copied, not encoded again.
use super::In;
use crate::{Error, Result, buf::Writer};
use horizon_world::nbt::{self, Tag};

/// `FriendlyByteBuf.readNbt`'s quota, as a cap on the bytes of one tag.
fn limits() -> nbt::Limits {
    nbt::Limits {
        max_bytes: 2 * 1024 * 1024,
        ..nbt::Limits::default()
    }
}

fn error(e: nbt::Error) -> Error {
    Error::new(format!("NBT: {e}"))
}

/// A named tag off the old wire, read. `End` is "no tag".
pub fn read(i: &mut In) -> Result<Tag> {
    let (named, used) = nbt::read_named(i.ahead(), limits()).map_err(error)?;
    i.bytes(used)?;
    Ok(named.tag)
}

/// A named tag off the old wire, copied as 1.20.2's network form.
pub fn to_network(i: &mut In, w: &mut Writer) -> Result<()> {
    let bytes = i.ahead();
    let (_, used) = nbt::read_named(bytes, limits()).map_err(error)?;
    w.u8(bytes[0]);
    if bytes[0] != 0 {
        let name = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        w.bytes(&bytes[3 + name..used]);
    }
    i.bytes(used)?;
    Ok(())
}

/// A network tag from Horizon, copied as the old wire's named form.
pub fn to_named(i: &mut In, w: &mut Writer) -> Result<()> {
    let bytes = i.ahead();
    let (_, used) = nbt::read_network(bytes, limits()).map_err(error)?;
    w.u8(bytes[0]);
    if bytes[0] != 0 {
        w.u16(0);
        w.bytes(&bytes[1..used]);
    }
    i.bytes(used)?;
    Ok(())
}
