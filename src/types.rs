pub fn read_var_int(buf: &[u8]) -> Result<(usize, i32), ()> {
    for len in 1..=5.min(buf.len()) {

        let extra_byte = (buf[len - 1] & (1 << 7)) > 0;
        if extra_byte {
            continue;
        }

        let buf = &buf[..len];

        return match minecraft_protocol::decoder::var_int::decode(&mut &*buf) {
            Ok(val) => Ok((len, val)),
            Err(_) => Err(()),
        };
    }

    Err(())
}

pub fn encode_var_int(i: i32) -> Result<Vec<u8>, ()> {
    let mut buf = Vec::with_capacity(5);
    minecraft_protocol::encoder::var_int::encode(&i, &mut buf).map_err(|_| ())?;
    Ok(buf)
}
