use crate::explicit::Route;
use serde::{Deserialize, Serialize};
use std::{net::Ipv4Addr, ops::RangeInclusive};

pub const CAPTURE_METADATA: [&str; 6] = [
    "capture_start_utc",
    "elapsed_us",
    "sample_index",
    "connection_generation",
    "packet_sequence",
    "cip_sequence",
];
pub const MAX_ASSEMBLY_BYTES: usize = 509;
/// Bound cyclic UDP payloads to fit a normal Ethernet MTU.
pub const MAX_LARGE_ASSEMBLY_BYTES: usize = 1400;
pub const MAX_MAPPED_FIELDS: usize = 256;
pub const MAX_TIMEOUT_MULTIPLIER_CODE: u8 = 7;

fn default_timeout_multiplier_code() -> u8 {
    3
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ConnectionTimeout {
    pub multiplier_code: u8,
    pub factor: Option<u16>,
    pub requested_receive_watchdog_us: Option<u64>,
    pub receive_watchdog_us: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    EveryPacket,
    OnChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub device_ip: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub local_udp_port: u16,
    /// Optional CIP port/link path from device_ip, the first bridge.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub io_route: String,
    pub input_instance: u16,
    pub input_size: u16,
    /// Request T->O multicast. The adapter selects the group and port.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub input_multicast: bool,
    /// Presence requests variable-size T->O. The adapter sees input_size as the
    /// maximum and the scanner enforces this lower bound locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_min_size: Option<u16>,
    pub output_instance: u16,
    pub output_size: u16,
    /// Presence requests variable-size O->T with output_size as the maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_min_size: Option<u16>,
    pub config_instance: Option<u16>,
    pub config_size: u16,
    pub config_hex: String,
    pub output_hex: String,
    pub input_only: bool,
    /// Use the adapter's listen-only heartbeat connection point.
    /// This role requires input_only, multicast input and zero write payloads.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listen_only: bool,
    pub output_run_idle: bool,
    pub input_run_idle: bool,
    pub output_priority: u8,
    pub input_priority: u8,
    pub output_rpi_us: u32,
    pub input_rpi_us: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub large_forward_open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub electronic_key: Option<ElectronicKey>,
    #[serde(default = "default_timeout_multiplier_code")]
    pub timeout_multiplier_code: u8,
    pub selection: Selection,
    #[serde(
        default,
        rename = "input_fields",
        alias = "capture_fields",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub capture_fields: Vec<CaptureField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_fields: Vec<CaptureField>,
    pub log_fields: Option<Vec<String>>,
    #[serde(default)]
    pub capture_mode: CaptureMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionRole {
    InputOnly,
    ListenOnly,
    ExclusiveOwner,
}

impl ConnectionRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::InputOnly => "Input-only",
            Self::ListenOnly => "Listen-only",
            Self::ExclusiveOwner => "Exclusive owner",
        }
    }
}

/// CIP electronic key format 4. The target evaluates revision compatibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectronicKey {
    pub vendor: u16,
    pub device_type: u16,
    pub product: u16,
    pub major: u8,
    pub minor: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compatible: bool,
}

impl ElectronicKey {
    pub(crate) fn validate(self) -> Result<(), &'static str> {
        if self.vendor == 0 || !(1..=127).contains(&self.major) {
            return Err("Electronic key requires a nonzero vendor and major revision 1–127");
        }
        if self.compatible && (self.device_type == 0 || self.product == 0 || self.minor == 0) {
            return Err(
                "Compatible electronic key requires a nonzero device type, product code and minor revision",
            );
        }
        Ok(())
    }

    pub(crate) fn wire_major(self) -> u8 {
        self.major | (u8::from(self.compatible) << 7)
    }
}

impl std::fmt::Display for ElectronicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{} {}.{}",
            self.vendor, self.device_type, self.product, self.major, self.minor
        )?;
        if self.compatible {
            f.write_str(" (compatible mode)")?;
        }
        Ok(())
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            device_ip: Ipv4Addr::LOCALHOST,
            local_ip: Ipv4Addr::LOCALHOST,
            local_udp_port: 0,
            io_route: String::new(),
            input_instance: 101,
            input_size: 4,
            input_multicast: false,
            input_min_size: None,
            output_instance: 254,
            output_size: 0,
            output_min_size: None,
            config_instance: Some(1),
            config_size: 0,
            config_hex: String::new(),
            output_hex: String::new(),
            input_only: true,
            listen_only: false,
            output_run_idle: false,
            input_run_idle: false,
            output_priority: 1,
            input_priority: 1,
            capture_fields: Vec::new(),
            output_fields: Vec::new(),
            log_fields: None,
            capture_mode: CaptureMode::EveryPacket,
            output_rpi_us: 1000,
            input_rpi_us: 1000,
            large_forward_open: false,
            electronic_key: None,
            timeout_multiplier_code: default_timeout_multiplier_code(),
            selection: Selection::default(),
        }
    }
}
pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace()
        .map(|word| {
            let word = word.strip_prefix("0x").unwrap_or(word);
            if word.len() != 2 {
                return Err(format!("Expected a two-digit hex byte, got {word:?}"));
            }
            u8::from_str_radix(word, 16).map_err(|_| format!("Invalid hex byte {word:?}"))
        })
        .collect()
}
pub fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
impl Settings {
    pub fn connection_timeout(&self, actual_input_rpi_us: Option<u32>) -> ConnectionTimeout {
        let code = self.timeout_multiplier_code;
        let factor = (code <= MAX_TIMEOUT_MULTIPLIER_CODE).then(|| 4u16 << code);
        let watchdog = |rpi| {
            factor
                .filter(|_| rpi != 0)
                .map(|factor| u64::from(factor) * u64::from(rpi))
        };
        ConnectionTimeout {
            multiplier_code: code,
            factor,
            requested_receive_watchdog_us: watchdog(self.input_rpi_us),
            receive_watchdog_us: actual_input_rpi_us.and_then(watchdog),
        }
    }

    pub fn input_only_without_writes(&self) -> bool {
        self.input_only
            && self.output_size == 0
            && self.output_min_size.is_none()
            && self.config_size == 0
            && !self.output_run_idle
    }

    pub fn role(&self) -> ConnectionRole {
        if self.listen_only {
            ConnectionRole::ListenOnly
        } else if self.input_only {
            ConnectionRole::InputOnly
        } else {
            ConnectionRole::ExclusiveOwner
        }
    }

    pub fn set_role(&mut self, role: ConnectionRole) {
        self.input_only = role != ConnectionRole::ExclusiveOwner;
        self.listen_only = role == ConnectionRole::ListenOnly;
        if self.listen_only {
            self.input_multicast = true;
        }
    }

    pub fn supports_auto_reconnect(&self) -> bool {
        !self.listen_only && self.input_only_without_writes()
    }

    pub fn parsed_io_route(&self) -> Result<Option<Route>, String> {
        let route = self.io_route.trim();
        if route.is_empty() {
            return Ok(None);
        }
        route
            .parse()
            .map(Some)
            .map_err(|error| format!("Cyclic I/O route — {error}"))
    }

    /// Allowed application byte counts, excluding all connected I/O headers.
    pub fn input_lengths(&self) -> Result<RangeInclusive<usize>, String> {
        let min = usize::from(self.input_min_size.unwrap_or(self.input_size));
        let max = usize::from(self.input_size);
        if min == 0 || min > max || max > MAX_LARGE_ASSEMBLY_BYTES {
            return Err(format!(
                "Input requires 1 ≤ minimum bytes ≤ maximum bytes ≤ {MAX_LARGE_ASSEMBLY_BYTES}"
            ));
        }
        Ok(min..=max)
    }

    /// Allowed O->T application byte counts, excluding connected I/O headers.
    pub fn output_lengths(&self) -> Result<RangeInclusive<usize>, String> {
        let max = usize::from(self.output_size);
        let Some(min) = self.output_min_size.map(usize::from) else {
            return Ok(max..=max);
        };
        if self.input_only || max == 0 {
            return Err(
                "Variable-size output requires an owner connection and nonzero maximum bytes"
                    .into(),
            );
        }
        if min > max || max > MAX_LARGE_ASSEMBLY_BYTES {
            return Err(format!(
                "Output requires 0 ≤ minimum bytes ≤ maximum bytes ≤ {MAX_LARGE_ASSEMBLY_BYTES}"
            ));
        }
        Ok(min..=max)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.device_ip.is_unspecified()
            || self.device_ip.is_multicast()
            || self.device_ip.is_broadcast()
        {
            return Err("Enter a unicast adapter IPv4 address".into());
        }
        if self.local_ip.is_unspecified()
            || self.local_ip.is_multicast()
            || self.local_ip.is_broadcast()
        {
            return Err("Enter the scanner NIC's assigned IPv4 address".into());
        }
        let route = self.parsed_io_route()?;
        if route.is_some() && (!self.input_only_without_writes() || self.input_multicast) {
            return Err("Routed cyclic I/O requires a unicast input-only profile with zero write bytes and O->T Run/Idle disabled".into());
        }
        if self.input_instance == 0 || self.output_instance == 0 {
            return Err("Set both connection point instances".into());
        }
        let (open, limit) = if self.large_forward_open {
            ("Large Forward_Open", MAX_LARGE_ASSEMBLY_BYTES)
        } else {
            ("Standard Forward_Open", MAX_ASSEMBLY_BYTES)
        };
        if self.input_size == 0
            || self.input_size as usize + 4 * self.input_run_idle as usize > limit
            || self.output_size as usize + 4 * self.output_run_idle as usize > limit
        {
            return Err(format!(
                "{open} supports up to {limit} assembly and Run/Idle bytes per direction and requires a nonempty input assembly"
            ));
        }
        if let Some(key) = self.electronic_key {
            key.validate()?;
        }
        if self.input_priority > 3 || self.output_priority > 3 {
            return Err("Connection priorities must be 0–3".into());
        }
        if self.timeout_multiplier_code > MAX_TIMEOUT_MULTIPLIER_CODE {
            return Err("CIP timeout multiplier code must be 0–7".into());
        }
        if !(1000..=10_000_000).contains(&self.input_rpi_us)
            || !(1000..=10_000_000).contains(&self.output_rpi_us)
        {
            return Err("Both RPIs must be between 1,000 and 10,000,000 µs".into());
        }
        if self.listen_only && !(self.input_only_without_writes() && self.input_multicast) {
            return Err("Listen-only requires input_only and multicast input, zero output and configuration bytes, and O->T Run/Idle disabled".into());
        }
        if self.input_only && self.output_size != 0 {
            return Err(
                "Input-only requires a heartbeat connection with zero output assembly bytes".into(),
            );
        }
        self.configuration()?;
        let segment_bytes = |value| if value <= 255 { 2 } else { 4 };
        let path_bytes = route
            .as_ref()
            .map_or(0, |route| route.port_segments().len())
            + 2
            + 10 * usize::from(self.electronic_key.is_some())
            + self.config_instance.map_or(0, segment_bytes)
            + segment_bytes(self.output_instance)
            + segment_bytes(self.input_instance)
            + if self.config_size == 0 {
                0
            } else {
                2 + (self.config_size as usize).next_multiple_of(2)
            };
        if path_bytes > 510 {
            return Err("Encoded connection path exceeds 255 words".into());
        }
        self.output()?;
        self.selection.validate(*self.input_lengths()?.start())?;
        if !self.input_only {
            validate_fields(&self.output_fields, self.output_size as usize, false)?;
        }
        self.selected_fields()?;
        Ok(())
    }

    pub fn csv_fields(&self) -> Result<Vec<CaptureField>, String> {
        let fields = self.selected_fields()?;
        if fields.is_empty() {
            return Err("Select at least one input field for CSV capture".into());
        }
        Ok(fields)
    }

    fn selected_fields(&self) -> Result<Vec<CaptureField>, String> {
        let mut mapped = self.input_mapping()?;
        if let Some(names) = &self.log_fields {
            for (i, name) in names.iter().enumerate() {
                if !mapped.iter().any(|field| field.name == *name) {
                    return Err(format!("Unknown CSV input field {name:?}"));
                }
                if names[..i].contains(name) {
                    return Err(format!("Duplicate CSV input field {name:?}"));
                }
            }
        }
        mapped.retain(|field| self.logs_input(&field.name));
        Ok(mapped)
    }

    fn logs_input(&self, name: &str) -> bool {
        self.log_fields
            .as_ref()
            .is_none_or(|names| names.iter().any(|selected| selected == name))
    }

    /// Validate the readback fields against the assembly size, independently of CSV.
    pub fn input_mapping(&self) -> Result<Vec<CaptureField>, String> {
        let min = *self.input_lengths()?.start();
        validate_fields(&self.capture_fields, min, true)?;
        let mapped = if self.capture_fields.is_empty() {
            self.selection.validate(min)?;
            vec![CaptureField {
                name: "value".into(),
                selection: self.selection,
            }]
        } else {
            self.capture_fields.clone()
        };
        Ok(mapped)
    }

    /// Decode an assembly payload with the profile's mapping and CSV selection.
    pub fn preview_input(&self, bytes: &[u8]) -> Result<InputPreview, String> {
        let lengths = self.input_lengths()?;
        if !lengths.contains(&bytes.len()) {
            let expected = match self.input_min_size {
                Some(min) => format!("{min}–{}", self.input_size),
                None => self.input_size.to_string(),
            };
            return Err(format!(
                "Expected {expected} input payload bytes, got {}",
                bytes.len()
            ));
        }
        let mut fields = Vec::new();
        for field in self.input_mapping()? {
            let selection = field.selection;
            let value = selection
                .decode(bytes)
                .ok_or_else(|| format!("Cannot decode input field {:?}", field.name))?;
            let raw = &bytes[selection.offset..selection.offset + selection.width()?];
            fields.push(InputFieldPreview {
                included_in_csv: self.logs_input(&field.name),
                name: field.name,
                selection,
                raw_hex: hex(raw),
                value: value.to_string(),
            });
        }
        Ok(InputPreview {
            input_assembly: self.input_instance,
            input_size: self.input_size,
            input_min_size: self.input_min_size,
            payload_bytes: self.input_min_size.map(|_| bytes.len()),
            payload_hex: hex(bytes),
            fields,
        })
    }

    pub fn output(&self) -> Result<Vec<u8>, String> {
        let lengths = self.output_lengths()?;
        let data = parse_hex(&self.output_hex)?;
        if !lengths.contains(&data.len()) {
            return Err(if self.output_min_size.is_some() {
                format!(
                    "Staged output requires {}–{} bytes",
                    lengths.start(),
                    lengths.end()
                )
            } else {
                "Output bytes do not match the declared output size".into()
            });
        }
        Ok(data)
    }

    pub fn configuration(&self) -> Result<Vec<u8>, String> {
        if self.config_size > 490
            || (self.config_size != 0 && self.config_instance.unwrap_or(0) == 0)
        {
            return Err(
                "Configuration data requires an assembly instance and at most 490 bytes".into(),
            );
        }
        let bytes = parse_hex(&self.config_hex)?;
        if bytes.len() != usize::from(self.config_size) {
            return Err("Configuration bytes do not match the declared configuration size".into());
        }
        Ok(bytes)
    }

    /// Encode one field into the staged configuration bytes.
    pub fn stage_configuration(&mut self, field: Selection, value: &str) -> Result<(), String> {
        let mut bytes = self.configuration()?;
        field.encode(value, &mut bytes)?;
        self.config_hex = hex(&bytes);
        Ok(())
    }

    /// Encode one field into the staged buffer without sending any data.
    pub fn stage_output(&mut self, index: usize, value: &str) -> Result<(), String> {
        if self.input_only {
            return Err("The input-only profile has no writable outputs".into());
        }
        let field = self
            .output_fields
            .get(index)
            .ok_or("Unknown output field")?;
        let mut bytes = self.output()?;
        field.selection.encode(value, &mut bytes)?;
        self.output_hex = hex(&bytes);
        Ok(())
    }
}

fn validate_fields(fields: &[CaptureField], size: usize, csv_names: bool) -> Result<(), String> {
    if fields.len() > MAX_MAPPED_FIELDS {
        return Err(format!(
            "Use at most {MAX_MAPPED_FIELDS} mapped fields per assembly"
        ));
    }
    for (index, field) in fields.iter().enumerate() {
        let mut chars = field.name.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("Invalid capture field name {:?}", field.name));
        }
        if (csv_names
            && CAPTURE_METADATA
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&field.name)))
            || fields[..index]
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(&field.name))
        {
            return Err(format!("Duplicate capture field name {:?}", field.name));
        }
        field
            .selection
            .validate(size)
            .map_err(|error| format!("{}: {error}", field.name))?;
    }
    Ok(())
}

mod mapping;
pub use mapping::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_staging_validates_buffer_before_mutating() {
        let mut settings = Settings {
            config_instance: Some(1),
            config_size: 3,
            config_hex: "33 AA BB".into(),
            ..Settings::default()
        };
        let selection = Selection {
            offset: 1,
            kind: Kind::I16,
            ..Selection::default()
        };
        assert!(settings.stage_configuration(selection, "100000").is_err());
        assert_eq!(settings.config_hex, "33 AA BB");
        settings.stage_configuration(selection, "-300").unwrap();
        assert_eq!(settings.config_hex, "33 D4 FE");
        for instance in [None, Some(0)] {
            settings.config_instance = instance;
            assert!(settings.stage_configuration(selection, "0").is_err());
            assert_eq!(settings.config_hex, "33 D4 FE");
        }
    }

    #[test]
    fn timeout_codes_use_negotiated_input_interval_and_support_legacy_profiles() {
        let mut settings: Settings =
            toml::from_str(include_str!("../samples/loopback.toml")).unwrap();
        assert_eq!(settings.timeout_multiplier_code, 3);
        for (code, factor) in [4u16, 8, 16, 32, 64, 128, 256, 512].into_iter().enumerate() {
            settings.timeout_multiplier_code = code as u8;
            assert!(settings.validate().is_ok());
            let timeout = settings.connection_timeout(Some(1500));
            assert_eq!(timeout.factor, Some(factor));
            assert_eq!(
                timeout.requested_receive_watchdog_us,
                Some(u64::from(factor) * 1000)
            );
            assert_eq!(timeout.receive_watchdog_us, Some(u64::from(factor) * 1500));
        }
        settings.timeout_multiplier_code = 8;
        assert!(settings.validate().is_err());
        assert_eq!(
            settings.connection_timeout(Some(1500)).receive_watchdog_us,
            None
        );
    }

    #[test]
    fn decode_types_endian_bit_and_bounds() {
        let mut s = Selection {
            offset: 1,
            kind: Kind::I16,
            big_endian: true,
            bit: None,
            ..Selection::default()
        };
        assert_eq!(s.decode(&[0, 0xff, 0xfe]), Some(Value::Signed(-2)));
        s.big_endian = false;
        assert_eq!(s.decode(&[0, 0xff, 0xfe]), Some(Value::Signed(-257)));
        assert_eq!(s.decode(&[0, 0xff]), None);
        s.bit = Some(7);
        assert_eq!(s.decode(&[0, 0xff]), Some(Value::Unsigned(1)));
        s.bit = None;
        s.kind = Kind::F32;
        assert_eq!(s.decode(&[0, 0, 0, 128, 63]), Some(Value::Float32(1.0)));
        s.offset = usize::MAX;
        assert_eq!(s.decode(&[]), None);
    }

    #[test]
    fn mapped_output_encoding_and_swap_round_trip() {
        let mut bytes = [0xaa; 8];
        let bit = Selection {
            kind: Kind::Bool,
            bit: Some(2),
            ..Selection::default()
        };
        bit.encode("true", &mut bytes).unwrap();
        assert_eq!(bytes[0], 0xae);
        assert_eq!(bit.decode(&bytes), Some(Value::Bool(true)));
        assert!(bit.encode("2", &mut bytes).is_err());
        assert_eq!(bytes[0], 0xae);

        let mut float = Selection {
            offset: 1,
            kind: Kind::F32,
            big_endian: true,
            swap: ByteSwap::WordOrder,
            ..Selection::default()
        };
        float.encode("1.5", &mut bytes).unwrap();
        assert_eq!(&bytes[..6], &[0xae, 0, 0, 0x3f, 0xc0, 0xaa]);
        assert_eq!(float.decode(&bytes), Some(Value::Float32(1.5)));
        float.swap = ByteSwap::BytesInWords;
        float.encode("1.5", &mut bytes).unwrap();
        assert_eq!(&bytes[1..5], &[0xc0, 0x3f, 0, 0]);
        assert_eq!(float.decode(&bytes), Some(Value::Float32(1.5)));

        let text = Selection {
            offset: 5,
            kind: Kind::Utf8,
            string_length: Some(3),
            ..Selection::default()
        };
        text.encode("é", &mut bytes).unwrap();
        assert_eq!(&bytes[5..], &[0xc3, 0xa9, 0]);
        assert_eq!(text.decode(&bytes), Some(Value::Text("é".as_bytes())));
        assert!(text.encode("abcd", &mut bytes).is_err());
        assert!(text.encode("\0", &mut bytes).is_err());
        assert_eq!(&bytes[5..], &[0xc3, 0xa9, 0]);
    }

    #[test]
    fn mapped_profile_round_trips_with_only_selected_columns() {
        let settings: Settings =
            toml::from_str(include_str!("../samples/mapped-loopback.toml")).unwrap();
        settings.validate().unwrap();
        let saved = toml::to_string_pretty(&settings).unwrap();
        let reloaded: Settings = toml::from_str(&saved).unwrap();
        assert_eq!(reloaded.capture_mode, CaptureMode::OnChange);
        assert_eq!(reloaded.capture_fields.len(), 3);
        assert_eq!(
            reloaded
                .csv_fields()
                .unwrap()
                .into_iter()
                .map(|field| field.name)
                .collect::<Vec<_>>(),
            ["counter", "low_bit"]
        );
    }
}
