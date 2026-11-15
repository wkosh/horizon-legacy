//! What Horizon sends, numbered and laid out for a 1.20.2 server, as the old server reads it. The old
//! server is in play from `login success` on, so what Horizon sends while it configures goes as the play
//! packet of the same meaning; what the old protocol has no packet for is not sent, and the answers a
//! 1.20.2 server would give to it (a block change's sequence, a ping) are made here.
use super::{
    In, Serverbound, Wire, slot,
    tables::{v758, v764},
};
use crate::{Result, State, buf::Writer};

/// `ServerboundChatPacket`'s limit before 1.19.
const CHAT_LENGTH: usize = 256;

fn body(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    w.bytes(payload);
    w.data
}

fn sent(id: i32, payload: &[u8]) -> Result<Serverbound> {
    Ok(Serverbound {
        wire: vec![body(id, payload)],
        incoming: Vec::new(),
    })
}

/// The client's settings: 1.17.1 has no "allow server listings" at the end.
fn settings(wire: &Wire, payload: &[u8]) -> Result<Serverbound> {
    let payload = match payload.split_last() {
        Some((_, rest)) if wire.v1_17 => rest,
        _ => payload,
    };
    sent(v758::sb::SETTINGS, payload)
}

/// Chat and commands are one packet: the text, a command behind its slash.
fn chat(text: &str, command: bool) -> Result<Serverbound> {
    let text: String = command
        .then_some('/')
        .into_iter()
        .chain(text.chars())
        .take(CHAT_LENGTH)
        .collect();
    let mut w = Writer::new();
    w.string(&text);
    sent(v758::sb::CHAT, &w.data)
}

/// A block action without the sequence 1.19 ends it with. The old server acknowledges a dig with the
/// block it holds and a use with nothing, so the sequence is acknowledged here, at once: the block
/// changes the server sends for the action follow it.
fn sequenced(
    id: i32,
    payload: &[u8],
    fixed: impl FnOnce(&mut In) -> Result<()>,
) -> Result<Serverbound> {
    let mut i = In::new(payload);
    fixed(&mut i)?;
    let kept = i.position();
    let sequence = i.varint()?;
    i.end()?;
    let mut ack = Writer::new();
    ack.varint(sequence);
    Ok(Serverbound {
        wire: vec![body(id, &payload[..kept])],
        incoming: vec![(v764::cb::ACKNOWLEDGE_PLAYER_DIGGING, ack.data)],
    })
}

/// The beacon's effects: optional registry ids from 0 in 1.20.2, the effects' numbers from 1 before,
/// -1 for none.
fn beacon(payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    for _ in 0..2 {
        w.varint(if i.bool()? { i.varint()? + 1 } else { -1 });
    }
    i.end()?;
    Ok(w.data)
}

/// A container click: the slots it changed and the carried stack are the old server's items.
fn window_click(wire: &Wire, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.u8(i.u8()?);
    w.varint(i.varint()?);
    w.i16(i.i16()?);
    w.i8(i.i8()?);
    w.varint(i.varint()?);
    let changed = i.counted()?;
    w.varint(changed as i32);
    for _ in 0..changed {
        w.i16(i.i16()?);
        slot::down(wire, &mut i, &mut w)?;
    }
    slot::down(wire, &mut i, &mut w)?;
    i.end()?;
    Ok(w.data)
}

fn play(wire: &Wire, id: i32, payload: &[u8]) -> Result<Serverbound> {
    use v764::sb;
    match id {
        sb::CHAT_MESSAGE => return chat(&In::new(payload).string(CHAT_LENGTH)?, false),
        sb::CHAT_COMMAND => return chat(&In::new(payload).string(CHAT_LENGTH)?, true),
        sb::SETTINGS => return settings(wire, payload),
        sb::BLOCK_DIG => {
            return sequenced(v758::sb::BLOCK_DIG, payload, |i| {
                i.varint()?;
                i.bytes(8 + 1).map(drop)
            });
        }
        sb::BLOCK_PLACE => {
            return sequenced(v758::sb::BLOCK_PLACE, payload, |i| {
                i.varint()?;
                i.i64()?;
                i.varint()?;
                i.bytes(4 * 3 + 1).map(drop)
            });
        }
        sb::USE_ITEM => {
            return sequenced(v758::sb::USE_ITEM, payload, |i| i.varint().map(drop));
        }
        // The old protocol has no ping of the client's: answered as a server at no distance would.
        sb::PING_REQUEST => {
            return Ok(Serverbound {
                wire: Vec::new(),
                incoming: vec![(v764::cb::PING_RESPONSE, payload.to_vec())],
            });
        }
        _ => {}
    }
    let Some(mapped) = usize::try_from(id)
        .ok()
        .and_then(|id| wire.serverbound.get(id).copied().flatten())
    else {
        return Ok(Serverbound::default());
    };
    let mapped = i32::from(mapped);
    match id {
        sb::SET_BEACON_EFFECT => sent(mapped, &beacon(payload)?),
        sb::UPDATE_SIGN => {
            // Without "is the front text": a sign had one side.
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.i64(i.i64()?);
            i.bool()?;
            w.bytes(i.rest());
            sent(mapped, &w.data)
        }
        sb::WINDOW_CLICK => sent(mapped, &window_click(wire, payload)?),
        sb::SET_CREATIVE_SLOT => {
            let mut i = In::new(payload);
            let mut w = Writer::new();
            w.i16(i.i16()?);
            slot::down(wire, &mut i, &mut w)?;
            i.end()?;
            sent(mapped, &w.data)
        }
        _ => sent(mapped, payload),
    }
}

/// One packet Horizon sends in `state`, as bodies for the old server and what Horizon reads back.
pub fn translate(wire: &Wire, state: State, id: i32, payload: &[u8]) -> Result<Serverbound> {
    match state {
        State::Handshake | State::Status => sent(id, payload),
        State::Login => match id {
            // The name alone: the UUID came with 1.19.
            v764::login_sb::LOGIN_START => {
                let mut w = Writer::new();
                w.string(&In::new(payload).string(16)?);
                sent(id, &w.data)
            }
            // The old server is in play from `login success` on, with nothing to wait for.
            v764::login_sb::LOGIN_ACKNOWLEDGED => Ok(Serverbound::default()),
            _ => sent(id, payload),
        },
        State::Configuration => match id {
            v764::config_sb::SETTINGS => settings(wire, payload),
            v764::config_sb::CUSTOM_PAYLOAD => sent(v758::sb::CUSTOM_PAYLOAD, payload),
            v764::config_sb::KEEP_ALIVE => sent(v758::sb::KEEP_ALIVE, payload),
            v764::config_sb::PONG => sent(v758::sb::PONG, payload),
            v764::config_sb::RESOURCE_PACK_RECEIVE => {
                sent(v758::sb::RESOURCE_PACK_RECEIVE, payload)
            }
            // `finish_configuration` answers the front's own.
            _ => Ok(Serverbound::default()),
        },
        State::Play => play(wire, id, payload),
    }
}
