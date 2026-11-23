//! Optional Ethernet Link port settings and advertised capabilities.

use super::LinkInfo;
use crate::explicit::{SavedReadOutcome, le16, le32, tcpip_info::cip_string};
use serde::Serialize;

const CONTROL: u32 = 6;
const MEDIA_TYPE: u32 = 7;
const OPERATING_STATE: u32 = 8;
const ADMIN_STATE: u32 = 9;
const LABEL: u32 = 10;
const CAPABILITIES: u32 = 11;

pub(super) const CONTROL_ATTRIBUTE: (u32, &str) = (CONTROL, "Interface control");
pub(super) const TYPE_ATTRIBUTE: (u32, &str) = (MEDIA_TYPE, "Interface type");
pub(super) const STATE_ATTRIBUTE: (u32, &str) = (OPERATING_STATE, "Interface state");
pub(super) const ADMIN_ATTRIBUTE: (u32, &str) = (ADMIN_STATE, "Administrative state");
pub(super) const LABEL_ATTRIBUTE: (u32, &str) = (LABEL, "Interface label");
pub(super) const CAPABILITY_ATTRIBUTE: (u32, &str) = (CAPABILITIES, "Interface capabilities");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkPortDetails {
    pub control: Option<LinkControl>,
    pub interface_type: Option<u8>,
    pub operational_state: Option<u8>,
    pub administrative_state: Option<u8>,
    pub label: Option<String>,
    pub capabilities: Option<LinkCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkControl {
    pub control_bits: u16,
    pub auto_negotiate: bool,
    pub forced_full_duplex: Option<bool>,
    pub forced_speed_mbps: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkCapabilities {
    pub flags: u32,
    pub manual_setting_requires_reset: bool,
    pub auto_negotiate: bool,
    pub auto_mdix: bool,
    pub manual_speed_duplex: bool,
    pub speed_duplex_options: Vec<LinkSpeedDuplex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkSpeedDuplex {
    pub speed_mbps: u16,
    pub duplex_code: u8,
}

impl LinkInfo {
    /// Return the decoded port details when inspected. See the batch for each raw outcome.
    pub fn port_details(&self) -> Option<LinkPortDetails> {
        let entry = |id| {
            self.batch
                .results
                .iter()
                .find(|row| row.request.path.attribute == Some(id))
        };
        entry(CONTROL)?;
        let reply = |id| match &entry(id)?.outcome {
            SavedReadOutcome::Reply(data) => Some(data.as_slice()),
            _ => None,
        };
        Some(LinkPortDetails {
            control: reply(CONTROL).and_then(|bytes| control(bytes).ok()),
            interface_type: reply(MEDIA_TYPE).and_then(|bytes| code(bytes, "Interface type").ok()),
            operational_state: reply(OPERATING_STATE)
                .and_then(|bytes| code(bytes, "Interface state").ok()),
            administrative_state: reply(ADMIN_STATE)
                .and_then(|bytes| code(bytes, "Administrative state").ok()),
            label: reply(LABEL).and_then(|bytes| label(bytes).ok()),
            capabilities: reply(CAPABILITIES).and_then(|bytes| capabilities(bytes).ok()),
        })
    }
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    Some(match attribute {
        CONTROL => control(bytes).map(|control| {
            if let (Some(speed), Some(full_duplex)) =
                (control.forced_speed_mbps, control.forced_full_duplex)
            {
                let duplex = duplex_name(u8::from(full_duplex));
                format!(
                    "0x{:04X} · Forced {speed} Mb/s {duplex}",
                    control.control_bits
                )
            } else {
                format!("0x{:04X} · Auto-negotiation enabled", control.control_bits)
            }
        }),
        MEDIA_TYPE => code(bytes, "Interface type").map(|code| {
            describe_code(
                code,
                &["Unknown", "Internal", "Twisted-pair", "Optical fiber"],
            )
        }),
        OPERATING_STATE => code(bytes, "Interface state")
            .map(|code| describe_code(code, &["Unknown", "Operational", "Disabled", "Testing"])),
        ADMIN_STATE => code(bytes, "Administrative state")
            .map(|code| describe_code(code, &["Reserved", "Enabled", "Disabled"])),
        LABEL => label(bytes),
        CAPABILITIES => capabilities(bytes).map(|capabilities| {
            let options = capabilities
                .speed_duplex_options
                .iter()
                .map(|option| {
                    format!(
                        "{} Mb/s {}",
                        option.speed_mbps,
                        duplex_name(option.duplex_code)
                    )
                })
                .collect::<Vec<_>>();
            let mut description = format!("0x{:08X}", capabilities.flags);
            for (advertised, feature) in [
                (
                    capabilities.manual_setting_requires_reset,
                    "Manual change requires reset",
                ),
                (capabilities.auto_negotiate, "Auto-negotiation"),
                (capabilities.auto_mdix, "Auto-MDIX"),
                (capabilities.manual_speed_duplex, "Manual speed/duplex"),
            ] {
                if advertised {
                    description.push_str(" · ");
                    description.push_str(feature);
                }
            }
            if !options.is_empty() {
                description.push_str(" · ");
                description.push_str(&options.join(", "));
            }
            description
        }),
        _ => return None,
    })
}

fn control(bytes: &[u8]) -> Result<LinkControl, String> {
    let [flags_lo, flags_hi, speed_lo, speed_hi] = *bytes else {
        return Err("Interface control requires four bytes".into());
    };
    let control_bits = u16::from_le_bytes([flags_lo, flags_hi]);
    let auto_negotiate = control_bits & 1 != 0;
    Ok(LinkControl {
        control_bits,
        auto_negotiate,
        forced_full_duplex: (!auto_negotiate).then_some(control_bits & 2 != 0),
        forced_speed_mbps: (!auto_negotiate).then_some(u16::from_le_bytes([speed_lo, speed_hi])),
    })
}

fn code(bytes: &[u8], name: &str) -> Result<u8, String> {
    match bytes {
        [value] => Ok(*value),
        _ => Err(format!("{name} requires one byte")),
    }
}

fn describe_code(code: u8, names: &[&str]) -> String {
    match names.get(usize::from(code)) {
        Some(name) => format!("{name} ({code})"),
        None => format!("Code {code}"),
    }
}

fn duplex_name(code: u8) -> String {
    match code {
        0 => "half duplex".into(),
        1 => "full duplex".into(),
        code => format!("duplex code {code}"),
    }
}

fn label(bytes: &[u8]) -> Result<String, String> {
    let Some((&count, text)) = bytes.split_first().filter(|(count, _)| **count <= 64) else {
        return Err("Interface label requires a length of up to 64 bytes".into());
    };
    if text.len() == usize::from(count) {
        return Ok(String::from_utf8_lossy(text).into_owned());
    }
    if text.first() == Some(&0) {
        return cip_string(bytes, "Interface label");
    }
    Err("Interface label length does not match its payload".into())
}

fn capabilities(bytes: &[u8]) -> Result<LinkCapabilities, String> {
    let (header, options) = bytes
        .split_first_chunk::<5>()
        .ok_or("Interface capabilities require flags and a speed/duplex count")?;
    let count = usize::from(header[4]);
    if options.len() != count * 3 {
        return Err(format!(
            "Interface capabilities declare {count} speed/duplex entries, got {} bytes",
            options.len()
        ));
    }
    let flags = le32(header, 0);
    Ok(LinkCapabilities {
        flags,
        manual_setting_requires_reset: flags & 1 != 0,
        auto_negotiate: flags & 2 != 0,
        auto_mdix: flags & 4 != 0,
        manual_speed_duplex: flags & 8 != 0,
        speed_duplex_options: options
            .chunks_exact(3)
            .map(|bytes| LinkSpeedDuplex {
                speed_mbps: le16(bytes, 0),
                duplex_code: bytes[2],
            })
            .collect(),
    })
}
