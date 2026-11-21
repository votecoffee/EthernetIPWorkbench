//! Get_Attribute_List for a single CIP class or instance with documented reply widths.

use super::{
    EipError, ReadPath, Reader, ReplyHeader, Result, Route, RoutedReadError, cip_data,
    encode_read_path, le16, parse_id, protocol,
};
use serde::Serialize;
use std::{collections::HashSet, net::Ipv4Addr};

const SERVICE: u8 = 0x03;
const ATTRIBUTE_LIST_ERROR: u8 = 0x0a;
const MAX_ATTRIBUTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AttributeWidth {
    pub attribute: u16,
    pub bytes: u16,
}

pub struct AttributeListReport {
    pub adapter_or_bridge: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub path: ReadPath,
    pub route: Option<Route>,
    pub fields: Vec<AttributeWidth>,
    pub reply: std::result::Result<AttributeListReply, RoutedReadError>,
}

pub struct AttributeListReply {
    pub general_status: u8,
    pub extended_status: Vec<u16>,
    pub body: Vec<u8>,
    /// A rejected service has no decoded list. A malformed list retains its raw body.
    pub entries: std::result::Result<Option<Vec<AttributeListEntry>>, String>,
}

pub struct AttributeListEntry {
    pub attribute: u16,
    pub status: u8,
    pub data: Option<Vec<u8>>,
}

impl AttributeListReport {
    /// True when the target returned an intact list and every attribute succeeded.
    pub fn is_complete(&self) -> bool {
        self.reply.as_ref().is_ok_and(|reply| {
            reply.general_status == 0
                && matches!(&reply.entries, Ok(Some(entries)) if entries.iter().all(|entry| entry.status == 0))
        })
    }
}

/// Parse whitespace-separated attribute IDs and decimal reply lengths (for example, `1:2 6:4`).
pub fn parse_attribute_widths(input: &str) -> std::result::Result<Vec<AttributeWidth>, String> {
    let fields = input
        .split_whitespace()
        .take(MAX_ATTRIBUTES + 1)
        .map(|field| {
            let (id, width) = field.split_once(':').ok_or_else(|| {
                format!("Expected an attribute ID and reply byte count in {field:?}")
            })?;
            Ok::<_, String>(AttributeWidth {
                attribute: u16::try_from(parse_id(id)?)
                    .map_err(|_| format!("Attribute ID {id} exceeds 65535"))?,
                bytes: width
                    .parse()
                    .map_err(|_| format!("Invalid decimal reply byte count in {field:?}"))?,
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    validate_fields(&fields)?;
    Ok(fields)
}

fn validate_fields(fields: &[AttributeWidth]) -> std::result::Result<(), String> {
    if !(1..=MAX_ATTRIBUTES).contains(&fields.len()) {
        return Err(format!("Enter from 1 to {MAX_ATTRIBUTES} attributes"));
    }
    let mut ids = HashSet::with_capacity(fields.len());
    if fields
        .iter()
        .any(|field| field.attribute == 0 || !ids.insert(field.attribute))
    {
        return Err("Attribute IDs must be nonzero and unique".into());
    }
    let entry_bytes = fields
        .iter()
        .map(|field| usize::from(field.bytes) + 4)
        .sum::<usize>();
    // The SendRRData payload includes 16 CPF bytes, a 4-byte CIP header, and a 2-byte count.
    if entry_bytes + 22 > usize::from(u16::MAX) {
        return Err("Declared reply exceeds the EtherNet/IP encapsulation limit".into());
    }
    Ok(())
}

/// Read several attributes from one object with one unconnected explicit request.
pub fn read_attribute_list(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    class: u32,
    instance: u32,
    fields: Vec<AttributeWidth>,
    route: Option<Route>,
) -> std::result::Result<AttributeListReport, String> {
    let path = ReadPath {
        class,
        instance,
        attribute: None,
    };
    path.validate().map_err(str::to_owned)?;
    validate_fields(&fields)?;
    let mut request = encode_read_path(SERVICE, path);
    request.extend_from_slice(&(fields.len() as u16).to_le_bytes());
    for field in &fields {
        request.extend_from_slice(&field.attribute.to_le_bytes());
    }
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let session = reader
        .0
        .as_mut()
        .ok_or_else(|| EipError::Closed.to_string())?;
    let decode = |data: &[u8]| decode_reply(data, &fields);
    let reply = if let Some(route) = &route {
        session.routed(&request, route, decode)
    } else {
        session
            .unconnected(request, None)
            .and_then(|reply| decode(cip_data(&reply)?))
            .map_err(RoutedReadError::Exchange)
    };
    Ok(AttributeListReport {
        adapter_or_bridge: adapter,
        local_ip: local,
        path,
        route,
        fields,
        reply,
    })
}

fn decode_reply(data: &[u8], fields: &[AttributeWidth]) -> Result<AttributeListReply> {
    let header = ReplyHeader::parse(data)?;
    if data[0] != (SERVICE | 0x80) {
        return Err(protocol("Unexpected Get_Attribute_List reply service"));
    }
    let body = &data[header.body_offset..];
    let entries = if matches!(header.general_status, 0 | ATTRIBUTE_LIST_ERROR) {
        decode_entries(body, fields).map(Some)
    } else {
        Ok(None)
    };
    Ok(AttributeListReply {
        general_status: header.general_status,
        extended_status: header.extended_status,
        body: body.to_vec(),
        entries,
    })
}

fn decode_entries(
    bytes: &[u8],
    fields: &[AttributeWidth],
) -> std::result::Result<Vec<AttributeListEntry>, String> {
    let (count, mut remaining) = bytes
        .split_first_chunk()
        .ok_or("Attribute list is missing its count")?;
    let count = usize::from(u16::from_le_bytes(*count));
    if count != fields.len() {
        return Err(format!(
            "Attribute list returned {count} entries, expected {}",
            fields.len()
        ));
    }
    let mut entries = Vec::with_capacity(count);
    for field in fields {
        let (header, rest) = remaining.split_first_chunk::<4>().ok_or_else(|| {
            format!(
                "Missing status header for attribute 0x{:X}",
                field.attribute
            )
        })?;
        let attribute = le16(header, 0);
        let status = header[2];
        if attribute != field.attribute || header[3] != 0 {
            return Err(format!(
                "Unexpected entry header for attribute 0x{:X} (ID 0x{attribute:X}, reserved 0x{:02X})",
                field.attribute, header[3]
            ));
        }
        let width = if status == 0 {
            usize::from(field.bytes)
        } else {
            0
        };
        let (value, rest) = rest
            .split_at_checked(width)
            .ok_or_else(|| format!("Attribute 0x{attribute:X} needs {width} value bytes"))?;
        entries.push(AttributeListEntry {
            attribute,
            status,
            data: (status == 0).then(|| value.to_vec()),
        });
        remaining = rest;
    }
    if !remaining.is_empty() {
        return Err(format!(
            "{} unexpected bytes after the attribute list",
            remaining.len()
        ));
    }
    Ok(entries)
}
