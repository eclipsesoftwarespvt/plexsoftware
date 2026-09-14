use nbt::CompoundTag;

pub fn lobby_dimension(codec: &CompoundTag) -> CompoundTag {

    let dimension_types = match codec.get_compound_tag("minecraft:dimension_type") {
        Ok(types) => types,
        Err(_) => return lobby_default_dimension(),
    };

    let mut base = lobby_base_dimension(dimension_types);

    base.insert_i8("piglin_safe", 1);
    base.insert_f32("ambient_light", 0.0);

    base.insert_i8("respawn_anchor_works", 0);
    base.insert_i8("has_skylight", 0);
    base.insert_i8("bed_works", 0);
    base.insert_str("effects", "minecraft:the_end");
    base.insert_i64("fixed_time", 0);
    base.insert_i8("has_raids", 0);
    base.insert_i32("min_y", 0);
    base.insert_i32("height", 16);
    base.insert_i32("logical_height", 16);
    base.insert_f64("coordinate_scale", 1.0);
    base.insert_i8("ultrawarm", 0);
    base.insert_i8("has_ceiling", 0);

    base
}

fn lobby_base_dimension(dimension_types: &CompoundTag) -> CompoundTag {

    let preferred = vec![
        "minecraft:the_end",
        "minecraft:the_nether",
        "minecraft:the_overworld",
    ];

    let dimensions = dimension_types.get_compound_tag_vec("value").unwrap();

    for name in preferred {
        if let Some(dimension) = dimensions
            .iter()
            .find(|d| d.get_str("name").map(|n| n == name).unwrap_or(false))
        {
            if let Ok(dimension) = dimension.get_compound_tag("element") {
                return dimension.clone();
            }
        }
    }

    if let Some(dimension) = dimensions.first() {
        if let Ok(dimension) = dimension.get_compound_tag("element") {
            return dimension.clone();
        }
    }

    lobby_default_dimension()
}

pub fn default_dimension_codec() -> CompoundTag {
    snbt_to_compound_tag(include_str!("../../res/dimension_codec.snbt"))
}

fn lobby_default_dimension() -> CompoundTag {
    snbt_to_compound_tag(include_str!("../../res/dimension.snbt"))
}

fn snbt_to_compound_tag(data: &str) -> CompoundTag {
    use quartz_nbt::io::{write_nbt, Flavor};
    use quartz_nbt::snbt;

    let compound = snbt::parse(data).expect("failed to parse SNBT");

    let mut binary = Vec::new();
    write_nbt(&mut binary, None, &compound, Flavor::Uncompressed)
        .expect("failed to encode NBT CompoundTag as binary");

    bin_to_compound_tag(&binary)
}

fn bin_to_compound_tag(data: &[u8]) -> CompoundTag {
    use nbt::decode::read_compound_tag;
    read_compound_tag(&mut &*data).unwrap()
}
