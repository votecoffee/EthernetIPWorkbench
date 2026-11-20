use super::{
    AttributeWrite, Class3Size, Identity, IdentityPin, ReadPath, Route, WriteCurrentCheck,
    WriteInspection, WriteOptions, WriteReport,
};
use crate::{
    model::{hex, parse_hex},
    settings_file,
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, net::Ipv4Addr, path::Path};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// An offline write proposal with encoded wire bytes and optional value checks.
/// An identity pin must be supplied separately when applying it.
#[derive(Clone, Debug)]
pub struct WriteDraft {
    pub request: AttributeWrite,
    pub class3_size: Option<Class3Size>,
    pub expected_current: Option<Vec<u8>>,
    pub readback: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    adapter_or_bridge: Ipv4Addr,
    local_ip: Ipv4Addr,
    route: Option<Route>,
    value_hex: String,
    expected_current_hex: Option<String>,
    #[serde(default)]
    readback: bool,
    class3_size: Option<Class3Size>,
    path: ReadPath,
}

impl WriteDraft {
    pub fn validate(&self) -> Result<(), String> {
        match self.class3_size {
            Some(size) => self.request.validate_connected(size),
            None => self.request.validate(),
        }
    }

    /// Load and validate a draft without connecting.
    pub fn load(path: &Path) -> Result<Self, String> {
        let load = || -> Result<Self, String> {
            let mut text = String::new();
            File::open(path)
                .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text))
                .map_err(|error| error.to_string())?;
            check_file_size(&text)?;
            let doc: Document = toml::from_str(&text).map_err(|error| error.to_string())?;
            if doc.version != VERSION {
                return Err(format!("Unsupported write draft version {}", doc.version));
            }
            let draft = Self {
                request: AttributeWrite {
                    adapter_or_bridge: doc.adapter_or_bridge,
                    local_ip: doc.local_ip,
                    path: doc.path,
                    route: doc.route,
                    value: parse_hex(&doc.value_hex)
                        .map_err(|error| format!("Value bytes · {error}"))?,
                },
                class3_size: doc.class3_size,
                expected_current: doc
                    .expected_current_hex
                    .as_deref()
                    .map(parse_hex)
                    .transpose()
                    .map_err(|error| format!("Expected current bytes · {error}"))?,
                readback: doc.readback,
            };
            draft.validate()?;
            Ok(draft)
        };
        load().map_err(|error| format!("Load write draft {} · {error}", path.display()))
    }

    /// Save a validated draft to a new TOML file.
    pub fn save_new(&self, path: &Path) -> Result<(), String> {
        let save = || -> Result<(), String> {
            self.validate()?;
            let document = Document {
                version: VERSION,
                adapter_or_bridge: self.request.adapter_or_bridge,
                local_ip: self.request.local_ip,
                route: self.request.route.clone(),
                value_hex: hex(&self.request.value),
                expected_current_hex: self.expected_current.as_deref().map(hex),
                readback: self.readback,
                class3_size: self.class3_size,
                path: self.request.path,
            };
            let text = toml::to_string_pretty(&document).map_err(|error| error.to_string())?;
            check_file_size(&text)?;
            settings_file::write_new(path, &text).map_err(|error| error.to_string())
        };
        save().map_err(|error| format!("Save write draft {} · {error}", path.display()))
    }

    /// Read the identity for operator review using the draft's transport.
    pub fn read_target_identity(&self) -> Result<Identity, String> {
        match self.class3_size {
            Some(size) => self.request.read_target_identity_connected(size),
            None => self.request.read_target_identity(),
        }
    }

    /// Read identity and the current attribute bytes on a single read-only session.
    pub fn inspect_current(&self) -> Result<WriteInspection, String> {
        self.request.inspect_current(self.class3_size)
    }

    /// Compare the current attribute against the staged bytes after verifying the recorded device.
    pub fn check_current(&self, identity: IdentityPin) -> Result<WriteCurrentCheck, String> {
        self.request.check_current(self.class3_size, identity)
    }

    /// Check the supplied identity and send the staged write at most once.
    pub fn apply(&self, identity: IdentityPin) -> Result<WriteReport, String> {
        let options = WriteOptions {
            expected_current: self.expected_current.as_deref(),
            readback: self.readback,
        };
        match self.class3_size {
            Some(size) => self.request.apply_connected(identity, options, size),
            None => self.request.apply_with_options(identity, options),
        }
    }
}

fn check_file_size(text: &str) -> Result<(), String> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err("Write draft exceeds 1 MiB".into());
    }
    Ok(())
}
