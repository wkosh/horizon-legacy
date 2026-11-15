//! The legacy front: servers on 1.18.2 (protocol 758) and 1.17.1 (756), spoken to as if they ran 1.20.2.
//!
//! The adapter's steps translate a release at a time up from 1.20.2 (764). Before 764 the wire is of an
//! older kind: no configuration state (the play login carries the registries), network NBT with a root
//! name, entity data serializers numbered differently, a spawn packet per kind of entity, chat as one
//! plain packet. The front sits between the socket and those steps and turns each packet off the wire
//! into what a 1.20.2 server would have sent (and each packet Horizon sends, laid out for 1.20.2, into
//! what the old server reads), registry ids included, so nothing above it knows the server is older:
//!
//! - **login**: `login success` gains its property list; the server is then in play while Horizon
//!   configures, so play packets are held until the login packet, whose registry codec becomes the
//!   configuration's registry data, followed by `finish_configuration` and the 1.20.2 play login;
//! - **play**: packets with the same layout are renumbered; the others are rewritten ([`play`],
//!   [`chunk`], [`entity`], [`commands`]); 1.17.1's are brought to 1.18.2's on the way;
//! - **serverbound**: [`serverbound`] drops what the old server has no packet for, joins the chat
//!   packets into the old one, and takes the block-change sequences out, acknowledging them itself.
//!
//! The tables ([`tables`]) are generated from PrismarineJS/minecraft-data by
//! `tools/generate_legacy_front.py`.
mod chunk;
mod commands;
mod entity;
mod join;
mod nbt;
mod play;
mod serverbound;
mod slot;
pub mod tables;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{ServerVersion, ids::Remap};
use crate::{Error, Result, State, buf::Reader};
use horizon_world::nbt::Tag;
use tables::{v756, v758, v764};

/// What differs between the wires the front reads.
pub(crate) struct Wire {
    pub version: ServerVersion,
    /// 1.17.1: the chunk, block entity and effect packets of before 1.18.
    pub v1_17: bool,
    /// For each clientbound play id (1.18.2's numbering), 764's id of the packet with that name.
    pub clientbound: &'static [Option<u8>],
    /// For each of 764's serverbound play ids, this wire's id.
    pub serverbound: &'static [Option<u8>],
    pub block_states: u32,
    pub block_state: Remap,
    pub block: Remap,
    pub item: Remap,
    pub entity_type: Remap,
    pub particle_type: Remap,
    pub sound_event: Remap,
    pub block_entity_type: Remap,
    pub sounds_by_name: &'static [(u16, &'static str)],
    pub particles: Particles,
    /// `block_state.up` as a flat table, built on first use: a chunk column looks thousands up.
    states: OnceLock<Box<[u16]>>,
}

/// This wire's ids of the particles that carry options, -1 for one it lacks.
pub(crate) struct Particles {
    pub block: i32,
    pub block_marker: i32,
    pub falling_dust: i32,
    pub dust: i32,
    pub dust_color_transition: i32,
    pub item: i32,
    pub vibration: i32,
    pub barrier: i32,
    pub light: i32,
}

static WIRE_758: Wire = Wire {
    version: ServerVersion::V1_18_2,
    v1_17: false,
    clientbound: v758::CLIENTBOUND,
    serverbound: v758::SERVERBOUND,
    block_states: v758::BLOCK_STATES,
    block_state: v758::BLOCK_STATE,
    block: v758::BLOCK,
    item: v758::ITEM,
    entity_type: v758::ENTITY_TYPE,
    particle_type: v758::PARTICLE_TYPE,
    sound_event: v758::SOUND_EVENT,
    block_entity_type: v758::BLOCK_ENTITY_TYPE,
    sounds_by_name: v758::SOUNDS_BY_NAME,
    particles: Particles {
        block: v758::particle::BLOCK,
        block_marker: v758::particle::BLOCK_MARKER,
        falling_dust: v758::particle::FALLING_DUST,
        dust: v758::particle::DUST,
        dust_color_transition: v758::particle::DUST_COLOR_TRANSITION,
        item: v758::particle::ITEM,
        vibration: v758::particle::VIBRATION,
        barrier: v758::BARRIER_PARTICLE,
        light: v758::LIGHT_PARTICLE,
    },
    states: OnceLock::new(),
};

static WIRE_756: Wire = Wire {
    version: ServerVersion::V1_17_1,
    v1_17: true,
    // 1.17.1's ids are brought to 1.18.2's first (`v756::CLIENTBOUND_AS_758`).
    clientbound: v758::CLIENTBOUND,
    serverbound: v756::SERVERBOUND,
    block_states: v756::BLOCK_STATES,
    block_state: v756::BLOCK_STATE,
    block: v756::BLOCK,
    item: v756::ITEM,
    entity_type: v756::ENTITY_TYPE,
    particle_type: v756::PARTICLE_TYPE,
    sound_event: v756::SOUND_EVENT,
    block_entity_type: v756::BLOCK_ENTITY_TYPE,
    sounds_by_name: v756::SOUNDS_BY_NAME,
    particles: Particles {
        block: v756::particle::BLOCK,
        block_marker: v756::particle::BLOCK_MARKER,
        falling_dust: v756::particle::FALLING_DUST,
        dust: v756::particle::DUST,
        dust_color_transition: v756::particle::DUST_COLOR_TRANSITION,
        item: v756::particle::ITEM,
        vibration: v756::particle::VIBRATION,
        barrier: v756::BARRIER_PARTICLE,
        light: v756::LIGHT_PARTICLE,
    },
    states: OnceLock::new(),
};

/// No such state: the table's filler, which 764 reads as air.
const NO_STATE: u16 = 0;

impl Wire {
    /// 764's block state for this wire's `id`; air for an id the wire has no state for.
    #[inline]
    pub fn state(&self, id: i32) -> i32 {
        let table = self.states.get_or_init(|| {
            (0..self.block_states)
                .map(|id| {
                    self.block_state
                        .up(id as i32)
                        .and_then(|id| u16::try_from(id).ok())
                        .unwrap_or(NO_STATE)
                })
                .collect()
        });
        usize::try_from(id)
            .ok()
            .and_then(|id| table.get(id))
            .map_or(i32::from(NO_STATE), |&id| i32::from(id))
    }
}

/// A packet's payload being read, with the bytes behind the reader for what is copied as it is.
pub(crate) struct In<'a> {
    pub data: &'a [u8],
    pub r: Reader<'a>,
}
impl<'a> In<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            r: Reader::new(data),
        }
    }
    /// The bytes not read yet, without reading them.
    pub fn ahead(&self) -> &'a [u8] {
        &self.data[self.r.position()..]
    }
    /// A count of things that take a byte each at least: one more than the bytes left is refused.
    pub fn counted(&mut self) -> Result<usize> {
        let left = self.r.remaining();
        self.r.count(left)
    }
    /// Nothing may follow a packet's last field.
    pub fn end(&self) -> Result<()> {
        match self.r.remaining() {
            0 => Ok(()),
            n => Err(Error::new(format!("{n} bytes left over"))),
        }
    }
}
impl<'a> std::ops::Deref for In<'a> {
    type Target = Reader<'a>;
    fn deref(&self) -> &Reader<'a> {
        &self.r
    }
}
impl std::ops::DerefMut for In<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.r
    }
}

/// What the front remembers of the world the server describes.
#[derive(Default)]
pub(crate) struct World {
    /// The play login arrived: Horizon has left configuration.
    joined: bool,
    /// Play packets that came before the play login, in the server's numbering.
    held: Vec<(i32, Vec<u8>)>,
    /// Entity types (764's ids) by network id, for entity data laid out by type.
    pub kinds: HashMap<i32, i32>,
    /// The login codec's dimension types, as the server sent them: a login or respawn names its
    /// dimension type by its whole element.
    pub dimension_types: Vec<(String, Tag)>,
    /// How many biomes the codec synchronized: the width of a chunk's direct biome palette.
    pub biomes: usize,
    /// The current dimension's lowest block and height.
    pub min_y: i32,
    pub height: i32,
    /// 1.17.1: the light of columns whose chunk has not arrived (a light update comes first), as the
    /// bytes after its `trust edges` flag.
    pub light: HashMap<(i32, i32), Vec<u8>>,
    /// 1.17.1: the columns that have arrived, whose light updates go straight on.
    pub columns: HashSet<(i32, i32)>,
}

/// Packets the front made of one: for Horizon to read (764's id and layout), in order.
pub(crate) type Clientbound = Vec<(i32, Vec<u8>)>;

/// What became of a packet Horizon sent.
#[derive(Default)]
pub(crate) struct Serverbound {
    /// Bodies (id and payload) for the server, in order.
    pub wire: Vec<Vec<u8>>,
    /// Packets (764's id and layout) the server will not send itself, for Horizon to read next.
    pub incoming: Clientbound,
}

/// One connection's legacy front.
pub struct Front {
    wire: &'static Wire,
    /// The server left login for play: before 1.20.2 `login success` does that at once.
    play: AtomicBool,
    world: Mutex<World>,
}

impl Front {
    /// The front for a server on `version`; `None` for one the adapter's steps read directly.
    pub fn new(version: ServerVersion) -> Option<Self> {
        let wire = match version {
            ServerVersion::V1_18_2 => &WIRE_758,
            ServerVersion::V1_17_1 => &WIRE_756,
            _ => return None,
        };
        Some(Self {
            wire,
            play: AtomicBool::new(false),
            world: Mutex::new(World {
                height: 256,
                ..World::default()
            }),
        })
    }

    /// The release on the wire.
    pub fn version(&self) -> ServerVersion {
        self.wire.version
    }

    /// A packet the server sent while Horizon reads `state`, as a 1.20.2 server would have sent it: any
    /// number of packets, each read in the state the ones before it leave Horizon in.
    pub(crate) fn clientbound(&self, state: State, id: i32, payload: &[u8]) -> Result<Clientbound> {
        if !self.play.load(Ordering::Acquire) {
            if state == State::Login && id == v764::login_cb::SUCCESS {
                self.play.store(true, Ordering::Release);
                return Ok(vec![(id, join::login_success(payload)?)]);
            }
            // Status and the rest of login are 1.20.2's.
            return Ok(vec![(id, payload.to_vec())]);
        }
        let id = if self.wire.v1_17 {
            usize::try_from(id)
                .ok()
                .and_then(|id| v756::CLIENTBOUND_AS_758.get(id))
                .map_or(-1, |&id| i32::from(id))
        } else {
            id
        };
        let mut world = self.world.lock().unwrap();
        if !world.joined {
            return match id {
                v758::cb::LOGIN => {
                    let mut out = join::configuration(self.wire, &mut world, payload)?;
                    world.joined = true;
                    for (id, payload) in std::mem::take(&mut world.held) {
                        out.extend(translated(self.wire, &mut world, id, &payload));
                    }
                    Ok(out)
                }
                // Horizon is still configuring: the same reason, in that state's packet.
                v758::cb::KICK_DISCONNECT => {
                    Ok(vec![(v764::config_cb::DISCONNECT, payload.to_vec())])
                }
                _ => {
                    world.held.push((id, payload.to_vec()));
                    Ok(Vec::new())
                }
            };
        }
        Ok(translated(self.wire, &mut world, id, payload))
    }

    /// A packet Horizon sends in `state`, numbered and laid out for a 1.20.2 server, as this server
    /// reads it.
    pub(crate) fn serverbound(&self, state: State, id: i32, payload: &[u8]) -> Result<Serverbound> {
        serverbound::translate(self.wire, state, id, payload)
    }
}

/// A play packet (1.18.2's numbering) as 1.20.2 lays it out. One the front cannot read is dropped with
/// a warning: handing its bytes on would have them read as another layout.
fn translated(wire: &Wire, world: &mut World, id: i32, payload: &[u8]) -> Clientbound {
    match play::clientbound(wire, world, id, payload) {
        Ok(packets) => packets,
        Err(e) => {
            log::warn!(
                "Horizon: could not translate {} play packet {id:#04x}: {e}",
                wire.version.name()
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests;
