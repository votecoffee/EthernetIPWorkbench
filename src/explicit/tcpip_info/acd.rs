//! Address Conflict Detection attributes and the adapter's retained ARP evidence.

use super::{CAPABILITY, STATUS, TcpIpInfo, dword};
use crate::{explicit::link_info::format_mac, model::hex};
use serde::Serialize;
use std::net::Ipv4Addr;

const SELECT_ACD: u32 = 10;
const LAST_CONFLICT: u32 = 11;
pub(super) const SELECT_ATTRIBUTE: (u32, &str) =
    (SELECT_ACD, "Address conflict detection selection");
pub(super) const CONFLICT_ATTRIBUTE: (u32, &str) = (LAST_CONFLICT, "Last address conflict");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddressConflictDiagnostics {
    pub capable: Option<bool>,
    pub enabled: Option<bool>,
    pub conflict_detected: Option<bool>,
    pub fault: Option<bool>,
    pub last_conflict: Option<AddressConflictRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddressConflictRecord {
    pub activity_code: u8,
    pub remote_mac: Option<String>,
    pub arp_pdu_hex: String,
    pub arp_ipv4: Option<Ipv4Arp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ipv4Arp {
    pub opcode: u16,
    pub sender_ip: Ipv4Addr,
    pub target_ip: Ipv4Addr,
}

impl TcpIpInfo {
    /// Distinguish unsupported or unreadable attributes from reported false values.
    pub fn address_conflicts(&self) -> Option<AddressConflictDiagnostics> {
        self.entry(SELECT_ACD)?;
        let status = self.reply(STATUS).and_then(|bytes| dword(bytes).ok());
        Some(AddressConflictDiagnostics {
            capable: self
                .reply(CAPABILITY)
                .and_then(|bytes| dword(bytes).ok())
                .map(|flags| flags & 0x80 != 0),
            enabled: self.reply(SELECT_ACD).and_then(|bytes| enabled(bytes).ok()),
            conflict_detected: status.map(|flags| flags & 0x40 != 0),
            fault: status.map(|flags| flags & 0x80 != 0),
            last_conflict: self
                .reply(LAST_CONFLICT)
                .and_then(|bytes| last_conflict(bytes).ok()),
        })
    }
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        SELECT_ACD => enabled(bytes).map(|on| {
            if on {
                "Enabled".into()
            } else {
                "Disabled".into()
            }
        }),
        LAST_CONFLICT => last_conflict(bytes).map(|record| record.description()),
        _ => return None,
    })
}

fn enabled(bytes: &[u8]) -> Result<bool, String> {
    match bytes {
        [0] => Ok(false),
        [1] => Ok(true),
        [_] => Err("Address conflict detection selection requires a BOOL value of 0 or 1".into()),
        _ => Err("Address conflict detection selection requires one byte".into()),
    }
}

fn last_conflict(bytes: &[u8]) -> Result<AddressConflictRecord, String> {
    let bytes: &[u8; 35] = bytes.try_into().map_err(|_| {
        format!(
            "Last address conflict requires 35 bytes, got {}",
            bytes.len()
        )
    })?;
    let &[activity_code, a, b, c, d, e, f, ref arp @ ..] = bytes;
    Ok(AddressConflictRecord {
        activity_code,
        remote_mac: (activity_code != 0).then(|| format_mac(&[a, b, c, d, e, f])),
        arp_pdu_hex: hex(arp),
        arp_ipv4: (activity_code != 0).then(|| Ipv4Arp::decode(arp)).flatten(),
    })
}

impl Ipv4Arp {
    fn decode(bytes: &[u8; 28]) -> Option<Self> {
        // The retained ARP PDU is in network byte order, unlike the CIP attribute fields.
        if bytes[..6] != [0, 1, 8, 0, 6, 4] {
            return None;
        }
        Some(Self {
            opcode: u16::from_be_bytes([bytes[6], bytes[7]]),
            sender_ip: Ipv4Addr::from(<[u8; 4]>::try_from(&bytes[14..18]).ok()?),
            target_ip: Ipv4Addr::from(<[u8; 4]>::try_from(&bytes[24..28]).ok()?),
        })
    }
}

impl AddressConflictRecord {
    fn description(&self) -> String {
        let stage = match self.activity_code {
            0 => return "No conflict recorded".into(),
            1 => "IPv4 probing".into(),
            2 => "Ongoing detection".into(),
            3 => "Semi-active probing".into(),
            code => format!("Activity code {code}"),
        };
        let mut result = format!(
            "{stage} · Remote MAC {}",
            self.remote_mac.as_deref().unwrap_or("unavailable")
        );
        if let Some(arp) = &self.arp_ipv4 {
            result.push_str(&format!(
                " · ARP sender {} · Target {}",
                arp.sender_ip, arp.target_ip
            ));
        }
        result
    }
}
