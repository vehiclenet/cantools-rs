use std::{fs::File, io::Write, path::Path};

use cantools_core::{
    CanFrame, CanId, CaptureEvent, Direction, FdFlags, FrameClass, InterfaceRef, Timestamp,
};
use mf4_rs::{
    api::mdf::MDF,
    blocks::{
        channel_block::ChannelBlock,
        channel_group_block::ChannelGroupBlock,
        common::{BlockHeader, DataType},
        data_group_block::DataGroupBlock,
        header_block::HeaderBlock,
        identification_block::IdentificationBlock,
        text_block::TextBlock,
    },
    error::MdfError,
};

use crate::{CodecError, FidelityNote, ReadReport, Result, WriteReport};

const RECORD_SIZE: usize = 116;
const CHANNEL_GROUP_NAME: &str = "cantools.capture";
const INTERFACE_NAME_LEN: usize = 32;
const FLAG_DIRECTION_TX: u16 = 0x0001;
const FLAG_REMOTE: u16 = 0x0002;
const FLAG_ERROR: u16 = 0x0004;
const FLAG_FD: u16 = 0x0008;
const FLAG_BRS: u16 = 0x0010;
const FLAG_ESI: u16 = 0x0020;
const FLAG_EXTENDED_ID: u16 = 0x0040;

fn mdf_error(error: MdfError) -> CodecError {
    CodecError::Parse(error.to_string())
}

fn timestamp_to_ns(timestamp: Timestamp) -> u64 {
    timestamp.seconds.max(0) as u64 * 1_000_000_000 + u64::from(timestamp.nanos)
}

fn ns_to_timestamp(ns: u64) -> Timestamp {
    Timestamp {
        seconds: (ns / 1_000_000_000) as i64,
        nanos: (ns % 1_000_000_000) as u32,
    }
}

fn encode_record(
    event: &CaptureEvent,
    notes: &mut Vec<FidelityNote>,
    index: usize,
) -> [u8; RECORD_SIZE] {
    let mut record = [0_u8; RECORD_SIZE];
    record[..8].copy_from_slice(&timestamp_to_ns(event.timestamp).to_le_bytes());
    record[8..12].copy_from_slice(&event.frame.id.raw().to_le_bytes());

    let mut flags = 0_u16;
    if matches!(event.direction, Direction::Tx) {
        flags |= FLAG_DIRECTION_TX;
    }
    if event.frame.class == FrameClass::Remote {
        flags |= FLAG_REMOTE;
    }
    if event.frame.class == FrameClass::Error {
        flags |= FLAG_ERROR;
    }
    if event.frame.fd {
        flags |= FLAG_FD;
    }
    if event.frame.fd_flags.bit_rate_switch {
        flags |= FLAG_BRS;
    }
    if event.frame.fd_flags.error_state_indicator {
        flags |= FLAG_ESI;
    }
    if event.frame.id.is_extended() {
        flags |= FLAG_EXTENDED_ID;
    }
    record[12..14].copy_from_slice(&flags.to_le_bytes());
    record[14..18].copy_from_slice(&event.interface.index.unwrap_or(0).to_le_bytes());
    record[18] = event.frame.data.len().min(64) as u8;

    let interface_name = event.interface.name.as_deref().unwrap_or("");
    let name_bytes = interface_name.as_bytes();
    if name_bytes.len() > INTERFACE_NAME_LEN {
        notes.push(FidelityNote {
            event_index: Some(index),
            field: "interface_name",
            detail: format!("MF4 export truncated interface name to {INTERFACE_NAME_LEN} bytes"),
        });
    }
    let name_len = name_bytes.len().min(INTERFACE_NAME_LEN);
    record[20..20 + name_len].copy_from_slice(&name_bytes[..name_len]);

    let data_len = event.frame.data.len().min(64);
    record[52..52 + data_len].copy_from_slice(&event.frame.data[..data_len]);

    if !event.annotations.is_empty() {
        notes.push(FidelityNote {
            event_index: Some(index),
            field: "annotations",
            detail: "MF4 export omits structured annotations in the bootstrap layout".to_string(),
        });
    }

    record
}

fn decode_record(record: &[u8]) -> Result<CaptureEvent> {
    let timestamp_ns = u64::from_le_bytes(record[..8].try_into().expect("timestamp bytes"));
    let raw_id = u32::from_le_bytes(record[8..12].try_into().expect("id bytes"));
    let flags = u16::from_le_bytes(record[12..14].try_into().expect("flag bytes"));
    let ifindex = u32::from_le_bytes(record[14..18].try_into().expect("ifindex bytes"));
    let data_len = usize::from(record[18]).min(64);
    let interface_name = {
        let raw = &record[20..20 + INTERFACE_NAME_LEN];
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).to_string()
    };
    let data = record[52..52 + data_len].to_vec();

    let id = if flags & FLAG_EXTENDED_ID != 0 {
        CanId::extended(raw_id)?
    } else {
        CanId::standard(raw_id as u16)?
    };
    let class = if flags & FLAG_ERROR != 0 {
        FrameClass::Error
    } else if flags & FLAG_REMOTE != 0 {
        FrameClass::Remote
    } else {
        FrameClass::Data
    };
    let frame = CanFrame::new(
        id,
        class,
        data,
        flags & FLAG_FD != 0,
        FdFlags {
            bit_rate_switch: flags & FLAG_BRS != 0,
            error_state_indicator: flags & FLAG_ESI != 0,
        },
    )?;
    let interface = InterfaceRef {
        name: if interface_name.is_empty() {
            None
        } else {
            Some(interface_name)
        },
        index: Some(ifindex),
    };
    let direction = if flags & FLAG_DIRECTION_TX != 0 {
        Direction::Tx
    } else {
        Direction::Rx
    };

    Ok(CaptureEvent::new(
        ns_to_timestamp(timestamp_ns),
        interface,
        direction,
        frame,
    ))
}

/// Write a capture-focused MF4 file using a fixed record layout.
pub fn write_mf4(path: &Path, events: &[CaptureEvent]) -> Result<WriteReport> {
    let channel_defs = [
        ("timestamp_ns", 0_u32, 64_u32, DataType::UnsignedIntegerLE),
        ("can_id", 8_u32, 32_u32, DataType::UnsignedIntegerLE),
        ("flags", 12_u32, 16_u32, DataType::UnsignedIntegerLE),
        (
            "interface_index",
            14_u32,
            32_u32,
            DataType::UnsignedIntegerLE,
        ),
        ("data_len", 18_u32, 8_u32, DataType::UnsignedIntegerLE),
        ("interface_name", 20_u32, 256_u32, DataType::StringLatin1),
        ("data", 52_u32, 512_u32, DataType::ByteArray),
    ];

    let mut notes = Vec::new();
    let header_stub_bytes = HeaderBlock::default().to_bytes().map_err(mdf_error)?;
    let id_bytes = IdentificationBlock::default()
        .to_bytes()
        .map_err(mdf_error)?;
    let comment_bytes = TextBlock::new("cantools-rs capture-focused MF4 export")
        .to_bytes()
        .map_err(mdf_error)?;
    let group_name_bytes = TextBlock::new(CHANNEL_GROUP_NAME)
        .to_bytes()
        .map_err(mdf_error)?;

    let channel_group_addr = id_bytes.len() as u64 + header_stub_bytes.len() as u64;
    let data_group_addr = channel_group_addr + 104;
    let comment_addr = data_group_addr + 64;
    let group_name_addr = comment_addr + comment_bytes.len() as u64;
    let first_channel_addr = group_name_addr + group_name_bytes.len() as u64;

    let mut channel_bytes = Vec::new();
    let mut channel_data_size = 0_u64;
    for (index, (name, byte_offset, bit_count, data_type)) in channel_defs.iter().enumerate() {
        let name_bytes = TextBlock::new(name).to_bytes().map_err(mdf_error)?;
        let current_addr = first_channel_addr + channel_data_size;
        let next_ch_addr = if index + 1 < channel_defs.len() {
            current_addr + 160 + name_bytes.len() as u64
        } else {
            0
        };
        let channel = ChannelBlock {
            header: BlockHeader {
                id: "##CN".to_string(),
                reserved0: 0,
                block_len: 160,
                links_nr: 8,
            },
            next_ch_addr,
            component_addr: 0,
            name_addr: current_addr + 160,
            source_addr: 0,
            conversion_addr: 0,
            data: 0,
            unit_addr: 0,
            comment_addr: 0,
            channel_type: 0,
            sync_type: if *name == "timestamp_ns" { 1 } else { 0 },
            data_type: data_type.clone(),
            bit_offset: 0,
            byte_offset: *byte_offset,
            bit_count: *bit_count,
            flags: 0,
            pos_invalidation_bit: 0,
            precision: 0,
            reserved1: 0,
            attachment_nr: 0,
            min_raw_value: 0.0,
            max_raw_value: 0.0,
            lower_limit: 0.0,
            upper_limit: 0.0,
            lower_ext_limit: 0.0,
            upper_ext_limit: 0.0,
            name: None,
            conversion: None,
        };
        let bytes = channel.to_bytes().map_err(mdf_error)?;
        channel_data_size += bytes.len() as u64 + name_bytes.len() as u64;
        channel_bytes.push((bytes, name_bytes));
    }

    let data_addr = first_channel_addr + channel_data_size;
    let header = HeaderBlock {
        first_dg_addr: data_group_addr,
        comment_addr,
        abs_time: events
            .first()
            .map(|event| timestamp_to_ns(event.timestamp))
            .unwrap_or(0),
        time_flags: 2,
        ..HeaderBlock::default()
    };
    let channel_group = ChannelGroupBlock {
        first_ch_addr: first_channel_addr,
        acq_name_addr: group_name_addr,
        comment_addr,
        cycles_nr: events.len() as u64,
        samples_byte_nr: RECORD_SIZE as u32,
        ..ChannelGroupBlock::default()
    };
    let data_group = DataGroupBlock {
        first_cg_addr: channel_group_addr,
        data_block_addr: data_addr,
        ..DataGroupBlock::default()
    };

    let mut payload = Vec::with_capacity(events.len() * RECORD_SIZE);
    for (index, event) in events.iter().enumerate() {
        payload.extend_from_slice(&encode_record(event, &mut notes, index));
    }

    let mut file = File::create(path)?;
    file.write_all(&id_bytes)?;
    file.write_all(&header.to_bytes().map_err(mdf_error)?)?;
    file.write_all(&channel_group.to_bytes().map_err(mdf_error)?)?;
    file.write_all(&data_group.to_bytes().map_err(mdf_error)?)?;
    file.write_all(&comment_bytes)?;
    file.write_all(&group_name_bytes)?;
    for (channel, name) in &channel_bytes {
        file.write_all(channel)?;
        file.write_all(name)?;
    }
    let data_header = BlockHeader {
        id: "##DT".to_string(),
        reserved0: 0,
        block_len: (24 + payload.len()) as u64,
        links_nr: 0,
    };
    file.write_all(&data_header.to_bytes().map_err(mdf_error)?)?;
    file.write_all(&payload)?;
    file.flush()?;

    Ok(WriteReport { notes })
}

/// Read a capture-focused MF4 file into capture events.
pub fn read_mf4(path: &Path) -> Result<ReadReport> {
    let mdf = MDF::from_file(path.to_string_lossy().as_ref()).map_err(mdf_error)?;
    let mut events = Vec::new();

    for group in mdf.channel_groups() {
        if group.name().map_err(mdf_error)?.as_deref() != Some(CHANNEL_GROUP_NAME) {
            continue;
        }
        let record_size = group.raw_channel_group().block.samples_byte_nr as usize;
        let data_blocks = group
            .raw_data_group()
            .data_blocks(group.mmap())
            .map_err(mdf_error)?;
        for data_block in data_blocks {
            for record in data_block.records(record_size) {
                if record.len() >= RECORD_SIZE {
                    events.push(decode_record(record)?);
                }
            }
        }
    }

    Ok(ReadReport {
        events,
        notes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use cantools_core::{
        CanFrame, CanId, CaptureEvent, Direction, FdFlags, FrameClass, InterfaceRef, Timestamp,
    };

    use super::{read_mf4, write_mf4};

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn mf4_round_trip() {
        let path = temp_file("cantools-codec-roundtrip.mf4");
        let events = vec![
            CaptureEvent::new(
                Timestamp {
                    seconds: 10,
                    nanos: 25,
                },
                InterfaceRef {
                    name: Some("vcan0".to_string()),
                    index: Some(1),
                },
                Direction::Rx,
                CanFrame::new(
                    CanId::standard(0x321).expect("id"),
                    FrameClass::Data,
                    vec![1, 2, 3, 4],
                    false,
                    FdFlags::default(),
                )
                .expect("frame"),
            ),
            CaptureEvent::new(
                Timestamp {
                    seconds: 11,
                    nanos: 0,
                },
                InterfaceRef {
                    name: Some("vcan1".to_string()),
                    index: Some(2),
                },
                Direction::Tx,
                CanFrame::new(
                    CanId::extended(0x18daf110).expect("id"),
                    FrameClass::Data,
                    (0..24).collect(),
                    true,
                    FdFlags {
                        bit_rate_switch: true,
                        error_state_indicator: false,
                    },
                )
                .expect("frame"),
            ),
        ];

        write_mf4(&path, &events).expect("write mf4");
        let report = read_mf4(&path).expect("read mf4");
        fs::remove_file(&path).ok();

        assert_eq!(report.events, events);
    }
}
