use std::{
    fs::File,
    io::{BufReader, BufWriter, Cursor, Write},
    path::Path,
};

use ablf::{BlfFile, ObjectTypes};
use binrw::BinWrite;
use cantools_core::{
    CanFrame, CanId, CaptureEvent, Direction, FdFlags, FrameClass, InterfaceRef, Timestamp,
};

use crate::{CodecError, FidelityNote, ReadReport, Result, WriteReport};

const LOGG_SIGNATURE: &[u8; 4] = b"LOGG";
const LOBJ_SIGNATURE: &[u8; 4] = b"LOBJ";
const OBJECT_TYPE_CAN_MESSAGE2: u32 = 86;
const TIME_ONE_NANS: u32 = 0x0000_0002;
const CAN_FLAG_EXTENDED: u32 = 0x8000_0000;
const CAN_FLAG_REMOTE: u32 = 0x4000_0000;
const CAN_FLAG_ERROR: u32 = 0x2000_0000;
const MESSAGE_FLAG_TX: u8 = 0x01;
const MESSAGE_FLAG_FD: u8 = 0x02;
const MESSAGE_FLAG_BRS: u8 = 0x04;
const MESSAGE_FLAG_ESI: u8 = 0x08;

#[derive(BinWrite)]
#[bw(little)]
struct FileHeaderWrite {
    signature: [u8; 4],
    stats_size: u32,
    api_version: u32,
    application_id: u8,
    application_major: u8,
    application_minor: u8,
    application_build: u8,
    file_size: u64,
    uncompressed_size: u64,
    object_count: u32,
    objects_read: u32,
    measurement_start_time: [u16; 8],
    last_object_time: [u16; 8],
    reserved: [u32; 18],
}

#[derive(BinWrite)]
#[bw(little)]
struct ObjectBaseHeaderWrite {
    signature: [u8; 4],
    header_size: u16,
    header_version: u16,
    object_size: u32,
    object_type: u32,
}

#[derive(BinWrite)]
#[bw(little)]
struct ObjectHeaderV1Write {
    flags: u32,
    client_index: u16,
    version: u16,
    timestamp_ns: u64,
}

#[derive(BinWrite)]
#[bw(little)]
struct CanMessage2Write {
    header: ObjectHeaderV1Write,
    channel: u16,
    flags: u8,
    dlc: u8,
    id: u32,
    frame_length_ns: u32,
    bit_count: u8,
    ext_data_offset: u8,
    data: [u8; 8],
}

fn timestamp_to_ns(timestamp: Timestamp) -> u64 {
    timestamp.seconds.max(0) as u64 * 1_000_000_000 + u64::from(timestamp.nanos)
}

fn decode_id(raw: u32) -> Result<CanId> {
    let extended = raw & CAN_FLAG_EXTENDED != 0;
    let value = raw & 0x1fff_ffff;
    if extended {
        Ok(CanId::extended(value)?)
    } else {
        Ok(CanId::standard(value as u16)?)
    }
}

fn build_can_message(event: &CaptureEvent) -> CanMessage2Write {
    let mut raw_id = event.frame.id.raw();
    if event.frame.id.is_extended() {
        raw_id |= CAN_FLAG_EXTENDED;
    }
    if event.frame.class == FrameClass::Remote {
        raw_id |= CAN_FLAG_REMOTE;
    }
    if event.frame.class == FrameClass::Error {
        raw_id |= CAN_FLAG_ERROR;
    }

    let mut flags = 0_u8;
    if matches!(event.direction, Direction::Tx) {
        flags |= MESSAGE_FLAG_TX;
    }
    if event.frame.fd {
        flags |= MESSAGE_FLAG_FD;
    }
    if event.frame.fd_flags.bit_rate_switch {
        flags |= MESSAGE_FLAG_BRS;
    }
    if event.frame.fd_flags.error_state_indicator {
        flags |= MESSAGE_FLAG_ESI;
    }

    let mut data = [0_u8; 8];
    let data_len = event.frame.data.len().min(64);
    data[..data_len.min(8)].copy_from_slice(&event.frame.data[..data_len.min(8)]);

    CanMessage2Write {
        header: ObjectHeaderV1Write {
            flags: TIME_ONE_NANS,
            client_index: 0,
            version: 0,
            timestamp_ns: timestamp_to_ns(event.timestamp),
        },
        channel: event.interface.index.unwrap_or(1) as u16,
        flags,
        dlc: data_len as u8,
        id: raw_id,
        frame_length_ns: data_len as u32,
        bit_count: (data_len * 8) as u8,
        ext_data_offset: 0,
        data,
    }
}

/// Read a BLF file into capture events.
pub fn read_blf(path: &Path) -> Result<ReadReport> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let blf = BlfFile::from_reader(reader).map_err(|(error, _)| CodecError::Io(error))?;
    if !blf.is_valid() {
        return Err(CodecError::Parse(format!(
            "invalid BLF file {}",
            path.display()
        )));
    }

    let mut notes = Vec::new();
    let mut events = Vec::new();
    for object in blf.into_iter() {
        match object.data {
            ObjectTypes::CanMessage86(message) => {
                notes.push(FidelityNote {
                    event_index: Some(events.len()),
                    field: "payload",
                    detail: "BLF import is best-effort in the bootstrap implementation and may not preserve every payload/layout detail".to_string(),
                });
                let id = decode_id(message.id)?;
                let class = if message.id & CAN_FLAG_ERROR != 0 {
                    FrameClass::Error
                } else if message.id & CAN_FLAG_REMOTE != 0 {
                    FrameClass::Remote
                } else {
                    FrameClass::Data
                };
                let data_len = usize::from(message.dlc.max(message.bit_count / 8)).min(8);
                let frame = CanFrame::new(
                    id,
                    class,
                    message.data[..data_len.min(8)].to_vec(),
                    false,
                    FdFlags {
                        bit_rate_switch: false,
                        error_state_indicator: false,
                    },
                )?;
                let timestamp = Timestamp {
                    seconds: (message.header.timestamp_ns / 1_000_000_000) as i64,
                    nanos: (message.header.timestamp_ns % 1_000_000_000) as u32,
                };
                let interface = InterfaceRef {
                    name: None,
                    index: Some(u32::from(message.channel)),
                };
                let direction = if message.flags & MESSAGE_FLAG_TX != 0 {
                    Direction::Tx
                } else {
                    Direction::Rx
                };
                events.push(CaptureEvent::new(timestamp, interface, direction, frame));
            }
            other => notes.push(FidelityNote {
                event_index: None,
                field: "object_type",
                detail: format!("ignored BLF object {other:?}"),
            }),
        }
    }

    Ok(ReadReport { events, notes })
}

/// Write a BLF file using a single uncompressed log container.
pub fn write_blf(path: &Path, events: &[CaptureEvent]) -> Result<WriteReport> {
    let mut notes = Vec::new();
    let mut inner_objects = Vec::new();

    for (index, event) in events.iter().enumerate() {
        if !event.annotations.is_empty() {
            notes.push(FidelityNote {
                event_index: Some(index),
                field: "annotations",
                detail: "BLF export omits structured annotations".to_string(),
            });
        }
        notes.push(FidelityNote {
            event_index: Some(index),
            field: "payload",
            detail: "BLF export is best-effort in the bootstrap implementation and may not preserve every payload/layout detail".to_string(),
        });

        let payload = {
            let mut bytes = Cursor::new(Vec::new());
            build_can_message(event)
                .write(&mut bytes)
                .map_err(|error| {
                    CodecError::Parse(format!("failed to encode BLF CAN message: {error}"))
                })?;
            bytes.into_inner()
        };

        let mut object_bytes = Cursor::new(Vec::new());
        ObjectBaseHeaderWrite {
            signature: *LOBJ_SIGNATURE,
            header_size: 16,
            header_version: 1,
            object_size: (16 + payload.len()) as u32,
            object_type: OBJECT_TYPE_CAN_MESSAGE2,
        }
        .write(&mut object_bytes)
        .map_err(|error| {
            CodecError::Parse(format!("failed to encode BLF object header: {error}"))
        })?;
        object_bytes.write_all(&payload)?;
        inner_objects.extend_from_slice(&object_bytes.into_inner());
    }

    let file_size = 144 + inner_objects.len();
    let file_header = FileHeaderWrite {
        signature: *LOGG_SIGNATURE,
        stats_size: 144,
        api_version: 0,
        application_id: 0,
        application_major: 0,
        application_minor: 1,
        application_build: 0,
        file_size: file_size as u64,
        uncompressed_size: inner_objects.len() as u64,
        object_count: events.len() as u32,
        objects_read: 0,
        measurement_start_time: [0; 8],
        last_object_time: [0; 8],
        reserved: [0; 18],
    };

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    file_header
        .write(&mut writer)
        .map_err(|error| CodecError::Parse(format!("failed to encode BLF file header: {error}")))?;
    writer.write_all(&inner_objects)?;
    writer.flush()?;

    Ok(WriteReport { notes })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use cantools_core::{
        CanFrame, CanId, CaptureEvent, Direction, FdFlags, FrameClass, InterfaceRef, Timestamp,
    };

    use super::{read_blf, write_blf};

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn blf_round_trip() {
        let path = temp_file("cantools-codec-roundtrip.blf");
        let event = CaptureEvent::new(
            Timestamp {
                seconds: 2,
                nanos: 5,
            },
            InterfaceRef {
                name: Some("can0".to_string()),
                index: Some(2),
            },
            Direction::Tx,
            CanFrame::new(
                CanId::extended(0x18daf110).expect("id"),
                FrameClass::Data,
                vec![0x02, 0x10, 0x03],
                false,
                FdFlags::default(),
            )
            .expect("frame"),
        );

        write_blf(&path, std::slice::from_ref(&event)).expect("write blf");
        let report = read_blf(&path).expect("read blf");
        fs::remove_file(&path).ok();

        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].frame.id, event.frame.id);
        assert_eq!(report.events[0].direction, event.direction);
        assert!(!report.notes.is_empty());
    }
}
