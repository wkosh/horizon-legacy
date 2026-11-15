//! Joining: before 1.20.2 there is no configuration state. `login success` puts the server in play, and
//! its play login carries the registry codec (dimension types and biomes) and names the dimension type by
//! its whole element. The front makes 1.20.2's configuration of that login: the registry data, the
//! enabled features, `finish_configuration`, then the play login as 1.20.2 lays it out.
use super::{Clientbound, In, Wire, World, nbt, tables::v764};
use crate::{Error, Result, buf::Writer};
use horizon_world::nbt::{JavaString, Tag};

/// `login success` with the property list 1.19 added (empty: the old packet has none).
pub fn login_success(payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let mut w = Writer::new();
    w.uuid(i.uuid()?);
    w.string(&i.string(16)?);
    i.end()?;
    w.varint(0);
    Ok(w.data)
}

fn string(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn int(tag: Option<&Tag>) -> Option<i32> {
    match tag {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        Some(Tag::Short(v)) => Some(i32::from(*v)),
        _ => None,
    }
}

/// A codec registry's entries by id, gaps filled with the first entry under a name of their own:
/// 1.20.2's registry data has an entry for every id.
fn entries(codec: &Tag, registry: &str) -> Vec<(String, Tag)> {
    let Some(Tag::List(list)) = codec.get(registry).and_then(|r| r.get("value")) else {
        return Vec::new();
    };
    let mut out: Vec<Option<(String, Tag)>> = Vec::new();
    for entry in list {
        let (Some(name), Some(id)) = (string(entry.get("name")), int(entry.get("id"))) else {
            continue;
        };
        let Ok(id) = usize::try_from(id) else {
            continue;
        };
        if id >= 65536 {
            continue;
        }
        if out.len() <= id {
            out.resize(id + 1, None);
        }
        out[id] = Some((name, entry.get("element").cloned().unwrap_or(Tag::End)));
    }
    let filler = out.iter().flatten().next().cloned();
    out.into_iter()
        .enumerate()
        .filter_map(|(id, entry)| {
            entry.or_else(|| {
                let (_, element) = filler.clone()?;
                Some((format!("minecraft:legacy_unused_{id}"), element))
            })
        })
        .collect()
}

/// A dimension type element as 1.19 reads it: the monster spawn light fields it added, and the
/// infiniburn tag written as a tag (1.18.2 put the `#` there).
fn dimension_type(element: &Tag) -> Tag {
    let mut element = element.clone();
    if let Tag::Compound(map) = &mut element {
        if let Some(Tag::String(tag)) = map.get_mut(&JavaString::from("infiniburn")) {
            let name = tag.to_string_lossy();
            if !name.starts_with('#') {
                *tag = JavaString::from(format!("#{name}").as_str());
            }
        }
        map.entry(JavaString::from("monster_spawn_light_level"))
            .or_insert(Tag::Int(0));
        map.entry(JavaString::from("monster_spawn_block_light_limit"))
            .or_insert(Tag::Int(0));
    }
    element
}

/// A biome element as 1.19.4 reads it: `has_precipitation` for the precipitation's name, and without
/// the category (1.19) and the depth and scale (1.18) the client no longer knows.
fn biome(element: &Tag) -> Tag {
    let mut element = element.clone();
    if let Tag::Compound(map) = &mut element {
        let precipitation = string(map.get(&JavaString::from("precipitation")));
        for gone in ["precipitation", "category", "depth", "scale"] {
            map.remove(&JavaString::from(gone));
        }
        map.entry(JavaString::from("has_precipitation"))
            .or_insert(Tag::Byte(i8::from(
                precipitation.as_deref().is_some_and(|p| p != "none"),
            )));
    }
    element
}

fn registry<'a>(name: &str, entries: impl IntoIterator<Item = (&'a str, Option<Tag>)>) -> Tag {
    let value = entries
        .into_iter()
        .enumerate()
        .map(|(id, (entry, element))| {
            let mut fields = vec![
                ("name", Tag::String(JavaString::from(entry))),
                ("id", Tag::Int(id as i32)),
            ];
            if let Some(element) = element {
                fields.push(("element", element));
            }
            Tag::compound(fields)
        })
        .collect();
    Tag::compound([
        ("type", Tag::String(JavaString::from(name))),
        ("value", Tag::List(value)),
    ])
}

/// The registries a 1.20.2 server synchronizes, from the login codec. Damage and chat types did not
/// exist: they are named without data, which Horizon reads from its own copy as for a pack both sides
/// know.
fn registry_data(world: &mut World, codec: &Tag) -> Tag {
    let dimension_types = entries(codec, "minecraft:dimension_type");
    let biomes = entries(codec, "minecraft:worldgen/biome");
    let tag = Tag::compound([
        (
            "minecraft:dimension_type",
            registry(
                "minecraft:dimension_type",
                dimension_types
                    .iter()
                    .map(|(name, element)| (name.as_str(), Some(dimension_type(element)))),
            ),
        ),
        (
            "minecraft:worldgen/biome",
            registry(
                "minecraft:worldgen/biome",
                biomes
                    .iter()
                    .map(|(name, element)| (name.as_str(), Some(biome(element)))),
            ),
        ),
        (
            "minecraft:damage_type",
            registry(
                "minecraft:damage_type",
                v764::DAMAGE_TYPES.iter().map(|name| (*name, None)),
            ),
        ),
        (
            "minecraft:chat_type",
            registry(
                "minecraft:chat_type",
                v764::CHAT_TYPES.iter().map(|name| (*name, None)),
            ),
        ),
    ]);
    world.biomes = biomes.len();
    world.dimension_types = dimension_types;
    tag
}

impl World {
    /// The name of the dimension type a login or respawn gives as its element, and the world's shape
    /// from it. An element the codec does not hold (a proxy that rewrites one of the two) is looked up
    /// by the level's name, then by shape.
    fn enter(&mut self, element: &Tag, level: &str) -> String {
        self.min_y = int(element.get("min_y")).unwrap_or(0);
        self.height = int(element.get("height")).unwrap_or(256);
        self.light.clear();
        self.columns.clear();
        self.kinds.clear();
        let shape = |e: &Tag| {
            (
                int(e.get("min_y")),
                int(e.get("height")),
                e.get("effects").cloned(),
            )
        };
        self.dimension_types
            .iter()
            .find(|(_, known)| known == element)
            .or_else(|| self.dimension_types.iter().find(|(name, _)| name == level))
            .or_else(|| {
                self.dimension_types
                    .iter()
                    .find(|(_, known)| shape(known) == shape(element))
            })
            .or_else(|| self.dimension_types.first())
            .map_or_else(
                || "minecraft:overworld".to_owned(),
                |(name, _)| name.clone(),
            )
    }
}

/// The play login, read. 1.17.1's has no simulation distance.
struct Login {
    entity: i32,
    hardcore: bool,
    game_mode: u8,
    previous_game_mode: i8,
    levels: Vec<String>,
    codec: Tag,
    dimension: Tag,
    level: String,
    seed: i64,
    max_players: i32,
    view_distance: i32,
    simulation_distance: i32,
    reduced_debug: bool,
    respawn_screen: bool,
    debug: bool,
    flat: bool,
}

fn read_login(wire: &Wire, payload: &[u8]) -> Result<Login> {
    let mut i = In::new(payload);
    let entity = i.i32()?;
    let hardcore = i.bool()?;
    let game_mode = i.u8()?;
    let previous_game_mode = i.i8()?;
    let count = i.counted()?;
    let mut levels = Vec::with_capacity(count);
    for _ in 0..count {
        levels.push(i.string(32767)?);
    }
    let codec = nbt::read(&mut i)?;
    let dimension = nbt::read(&mut i)?;
    let level = i.string(32767)?;
    let seed = i.i64()?;
    let max_players = i.varint()?;
    let view_distance = i.varint()?;
    let simulation_distance = if wire.v1_17 {
        view_distance
    } else {
        i.varint()?
    };
    let login = Login {
        entity,
        hardcore,
        game_mode,
        previous_game_mode,
        levels,
        codec,
        dimension,
        level,
        seed,
        max_players,
        view_distance,
        simulation_distance,
        reduced_debug: i.bool()?,
        respawn_screen: i.bool()?,
        debug: i.bool()?,
        flat: i.bool()?,
    };
    i.end()?;
    Ok(login)
}

/// 1.20.2's play login.
fn write_login(world: &mut World, login: &Login) -> Vec<u8> {
    let dimension_type = world.enter(&login.dimension, &login.level);
    world.kinds.insert(login.entity, v764::entity::PLAYER);
    let mut w = Writer::new();
    w.i32(login.entity);
    w.bool(login.hardcore);
    w.varint(login.levels.len() as i32);
    for level in &login.levels {
        w.string(level);
    }
    w.varint(login.max_players);
    w.varint(login.view_distance);
    w.varint(login.simulation_distance);
    w.bool(login.reduced_debug);
    w.bool(login.respawn_screen);
    w.bool(false); // Limited crafting came with a game event before 1.20.2.
    w.string(&dimension_type);
    w.string(&login.level);
    w.i64(login.seed);
    w.u8(login.game_mode);
    w.i8(login.previous_game_mode);
    w.bool(login.debug);
    w.bool(login.flat);
    w.bool(false); // No last death location before 1.19.
    w.varint(0); // Nor a portal cooldown before 1.20.
    w.data
}

/// The first play login: Horizon's whole configuration, then the login itself.
pub fn configuration(wire: &Wire, world: &mut World, payload: &[u8]) -> Result<Clientbound> {
    let login = read_login(wire, payload)?;
    if !matches!(login.codec, Tag::Compound(_)) {
        return Err(Error::new("login without a registry codec"));
    }
    let mut registries = Writer::new();
    registries.nbt(&registry_data(world, &login.codec));
    let mut features = Writer::new();
    features.varint(1);
    features.string("minecraft:vanilla");
    Ok(vec![
        (v764::config_cb::REGISTRY_DATA, registries.data),
        (v764::config_cb::FEATURE_FLAGS, features.data),
        (v764::config_cb::FINISH_CONFIGURATION, Vec::new()),
        (v764::cb::LOGIN, write_login(world, &login)),
    ])
}

/// A later play login (a proxy moving the player to another server): Horizon keeps the registries of
/// the first, as a 1.20.2 client does without a new configuration.
pub fn login(wire: &Wire, world: &mut World, payload: &[u8]) -> Result<Vec<u8>> {
    let login = read_login(wire, payload)?;
    Ok(write_login(world, &login))
}

/// A respawn: the dimension type by name, no last death location or portal cooldown, and which data
/// to keep as 1.19.3's flags (the old flag kept the attributes too).
pub fn respawn(world: &mut World, payload: &[u8]) -> Result<Vec<u8>> {
    let mut i = In::new(payload);
    let dimension = nbt::read(&mut i)?;
    let level = i.string(32767)?;
    let me: Vec<(i32, i32)> = world
        .kinds
        .iter()
        .filter(|&(_, &kind)| kind == v764::entity::PLAYER)
        .map(|(&id, &kind)| (id, kind))
        .collect();
    let dimension_type = world.enter(&dimension, &level);
    // Other players are spawned again; the player's own entity keeps its id.
    world.kinds.extend(me);
    let mut w = Writer::new();
    w.string(&dimension_type);
    w.string(&level);
    w.i64(i.i64()?);
    w.u8(i.u8()?);
    w.i8(i.i8()?);
    w.bool(i.bool()?);
    w.bool(i.bool()?);
    let keep_all = i.bool()?;
    i.end()?;
    w.bool(false);
    w.varint(0);
    const KEEP_ATTRIBUTES: u8 = 1;
    const KEEP_ENTITY_DATA: u8 = 2;
    w.u8(if keep_all {
        KEEP_ATTRIBUTES | KEEP_ENTITY_DATA
    } else {
        KEEP_ENTITY_DATA
    });
    Ok(w.data)
}
