use std::fmt::Debug;
use std::io::prelude::*;

use bytes::BytesMut;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use minecraft_protocol::encoder::Encoder;
use minecraft_protocol::version::PacketId;
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{ReadHalf, WriteHalf};

use crate::proto::client::Client;
use crate::proto::BUF_SIZE;
use crate::types;

pub struct RawPacket {

    pub id: u8,

    pub data: Vec<u8>,
}

impl RawPacket {

    pub fn new(id: u8, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    fn read_packet_id_data(mut buf: &[u8]) -> Result<Self, ()> {

        let (read, packet_id) = types::read_var_int(buf)?;
        buf = &buf[read..];

        Ok(Self::new(packet_id as u8, buf.to_vec()))
    }

    pub fn decode_with_len(client: &Client, mut buf: &[u8]) -> Result<Self, ()> {

        let (read, len) = types::read_var_int(buf)?;
        buf = &buf[read..][..len as usize];

        Self::decode_without_len(client, buf)
    }

    pub fn decode_without_len(client: &Client, mut buf: &[u8]) -> Result<Self, ()> {

        if !client.is_compressed() {

            return Self::read_packet_id_data(buf);
        }

        let (read, data_len) = types::read_var_int(buf)?;
        buf = &buf[read..];

        if data_len == 0 {
            return Self::read_packet_id_data(buf);
        }

        let mut decompressed = Vec::with_capacity(data_len as usize);
        ZlibDecoder::new(buf)
            .read_to_end(&mut decompressed)
            .map_err(|err| {
                error!(target: "plexpaper", "Packet decompression error: {}", err);
            })?;

        if decompressed.len() != data_len as usize {
            error!(target: "plexpaper", "Decompressed packet has different length than expected ({}b != {}b)", decompressed.len(), data_len);
            return Err(());
        }

        Self::read_packet_id_data(&decompressed)
    }

    pub fn encode_with_len(&self, client: &Client) -> Result<Vec<u8>, ()> {

        let mut payload = self.encode_without_len(client)?;

        let mut packet = types::encode_var_int(payload.len() as i32)?;
        packet.append(&mut payload);
        Ok(packet)
    }

    pub fn encode_without_len(&self, client: &Client) -> Result<Vec<u8>, ()> {
        let threshold = client.compressed();
        if threshold >= 0 {
            self.encode_compressed(threshold)
        } else {
            self.encode_uncompressed()
        }
    }

    fn encode_compressed(&self, threshold: i32) -> Result<Vec<u8>, ()> {

        let mut payload = types::encode_var_int(self.id as i32)?;
        payload.extend_from_slice(&self.data);

        let data_len = payload.len() as i32;
        let compress = data_len > threshold;
        let data_len_header = if compress { data_len } else { 0 };

        if compress {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&payload).map_err(|err| {
                error!(target: "plexpaper", "Failed to compress packet: {}", err);
            })?;
            payload = encoder.finish().map_err(|err| {
                error!(target: "plexpaper", "Failed to compress packet: {}", err);
            })?;
        }

        let mut packet = types::encode_var_int(data_len_header).unwrap();
        packet.append(&mut payload);

        Ok(packet)
    }

    fn encode_uncompressed(&self) -> Result<Vec<u8>, ()> {
        let mut packet = types::encode_var_int(self.id as i32)?;
        packet.extend_from_slice(&self.data);

        Ok(packet)
    }
}

pub async fn read_packet(
    client: &Client,
    buf: &mut BytesMut,
    stream: &mut ReadHalf<'_>,
) -> Result<Option<(RawPacket, Vec<u8>)>, ()> {

    while buf.len() < 2 {

        let mut tmp = Vec::with_capacity(BUF_SIZE);
        match stream.read_buf(&mut tmp).await {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
            Err(err) => {
                dbg!(err);
                return Err(());
            }
        }

        if tmp.is_empty() {
            return Ok(None);
        }
        buf.extend(tmp);
    }

    let (consumed, len) = match types::read_var_int(buf) {
        Ok(result) => result,
        Err(err) => {
            error!(target: "plexpaper", "Malformed packet, could not read packet length");
            return Err(err);
        }
    };

    while buf.len() < consumed + len as usize {

        let mut tmp = Vec::with_capacity(BUF_SIZE);
        match stream.read_buf(&mut tmp).await {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
            Err(err) => {
                dbg!(err);
                return Err(());
            }
        }

        if tmp.is_empty() {
            return Ok(None);
        }

        buf.extend(tmp);
    }

    let raw = buf.split_to(consumed + len as usize);
    let packet = RawPacket::decode_with_len(client, &raw)?;

    Ok(Some((packet, raw.to_vec())))
}

pub async fn write_packet(
    packet: impl PacketId + Encoder + Debug,
    client: &Client,
    writer: &mut WriteHalf<'_>,
) -> Result<(), ()> {
    let mut data = Vec::new();
    packet.encode(&mut data).map_err(|_| ())?;

    let response = RawPacket::new(packet.packet_id(), data).encode_with_len(client)?;
    writer.write_all(&response).await.map_err(|_| ())?;

    Ok(())
}
