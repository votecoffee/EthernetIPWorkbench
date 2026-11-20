use super::{
    AttributeWrite, Class3Size, IdentityPin, ReadPath, Route, WriteCurrentCheck, WriteDraft,
};
use crate::model::parse_hex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, fs::File, io::Read, net::Ipv4Addr, path::Path};

const MAX_RECEIPT_BYTES: u64 = 4 * 1024 * 1024;

/// The record of a prior explicit write attempt, usable for a read-only follow-up.
#[derive(Debug)]
pub struct SavedWriteReceipt {
    draft: WriteDraft,
    original: OriginalWrite,
    completed_utc: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedWriteOutcome {
    NotSent,
    Acknowledged,
    Rejected,
    RouterError,
    Unconfirmed,
}

impl fmt::Display for SavedWriteOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotSent => "Not sent",
            Self::Acknowledged => "Acknowledged",
            Self::Rejected => "Rejected by target",
            Self::RouterError => "Router error · Target outcome unconfirmed",
            Self::Unconfirmed => "Unconfirmed",
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct OriginalWrite {
    outcome: SavedWriteOutcome,
    expected_identity: IdentityPin,
}

#[derive(Deserialize)]
struct Document {
    adapter_or_bridge: Ipv4Addr,
    local_ip: Ipv4Addr,
    route: Option<Route>,
    path: ReadPath,
    service: String,
    value_hex: String,
    bytes: usize,
    class3_size: Option<Class3Size>,
    large_forward_open: Option<bool>,
    completed_utc: Option<String>,
    result: OriginalWrite,
}

impl SavedWriteReceipt {
    /// Validate a completed GUI or CLI write receipt without connecting to the recorded endpoint.
    pub fn load(path: &Path) -> Result<Self, String> {
        let load = || -> Result<Self, String> {
            let mut bytes = Vec::new();
            File::open(path)
                .and_then(|file| file.take(MAX_RECEIPT_BYTES + 1).read_to_end(&mut bytes))
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > MAX_RECEIPT_BYTES {
                return Err("Write receipt exceeds 4 MiB".into());
            }
            let doc: Document =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if doc.service != "Set_Attribute_Single" {
                return Err("Expected a Set_Attribute_Single receipt".into());
            }
            let value =
                parse_hex(&doc.value_hex).map_err(|error| format!("Value bytes · {error}"))?;
            if value.len() != doc.bytes {
                return Err("Write receipt byte count differs from its staged value".into());
            }
            if doc.large_forward_open != doc.class3_size.map(Class3Size::uses_large_forward_open) {
                return Err("Write receipt Class 3 size and open service disagree".into());
            }
            if let Some(time) = &doc.completed_utc {
                chrono::DateTime::parse_from_rfc3339(time)
                    .map_err(|error| format!("Completion time · {error}"))?;
            }
            let draft = WriteDraft {
                request: AttributeWrite {
                    adapter_or_bridge: doc.adapter_or_bridge,
                    local_ip: doc.local_ip,
                    route: doc.route,
                    path: doc.path,
                    value,
                },
                class3_size: doc.class3_size,
                expected_current: None,
                readback: false,
            };
            draft.validate()?;
            Ok(Self {
                draft,
                original: doc.result,
                completed_utc: doc.completed_utc,
            })
        };
        load().map_err(|error| format!("Load write receipt {} · {error}", path.display()))
    }

    pub fn request(&self) -> &AttributeWrite {
        &self.draft.request
    }

    pub fn class3_size(&self) -> Option<Class3Size> {
        self.draft.class3_size
    }

    pub fn expected_identity(&self) -> IdentityPin {
        self.original.expected_identity
    }

    pub fn outcome(&self) -> SavedWriteOutcome {
        self.original.outcome
    }

    pub fn completed_utc(&self) -> Option<&str> {
        self.completed_utc.as_deref()
    }

    /// Describe the original target, staged bytes and historical outcome.
    pub fn report_document(&self) -> Value {
        let mut report = self.draft.report_document();
        report["original_write"] = json!(self.original);
        if let Some(time) = &self.completed_utc {
            report["completed_utc"] = json!(time);
        }
        report
    }

    /// Check the original device identity before reading and comparing its current bytes.
    pub fn check_current(&self) -> Result<WriteCurrentCheck, String> {
        self.draft.check_current(self.original.expected_identity)
    }
}
