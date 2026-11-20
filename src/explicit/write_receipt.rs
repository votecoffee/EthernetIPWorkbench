use super::{
    AttributeWrite, Class3Size, IdentityPin, ReadPath, ReadbackOutcome, Route, WriteCurrentCheck,
    WriteDraft, WriteOutcome, WriteReport,
};
use crate::model::parse_hex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
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

/// A saved original receipt or an exported read-only comparison of the written value.
#[derive(Debug)]
pub struct SavedWriteRecord {
    receipt: SavedWriteReceipt,
    check: Option<WriteCurrentCheck>,
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

impl From<&WriteOutcome> for SavedWriteOutcome {
    fn from(outcome: &WriteOutcome) -> Self {
        match outcome {
            WriteOutcome::NotSent { .. } => Self::NotSent,
            WriteOutcome::Acknowledged { .. } => Self::Acknowledged,
            WriteOutcome::Rejected { .. } => Self::Rejected,
            WriteOutcome::RouterError { .. } => Self::RouterError,
            WriteOutcome::Unconfirmed { .. } => Self::Unconfirmed,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct OriginalWrite {
    outcome: SavedWriteOutcome,
    expected_identity: IdentityPin,
}

#[derive(Deserialize)]
struct Document<R> {
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
    result: R,
}

#[derive(Deserialize)]
struct RecordHeader {
    original_write: Option<OriginalWrite>,
}

impl SavedWriteReceipt {
    /// Build the follow-up reference from a completed write still in memory.
    pub fn from_report(draft: &WriteDraft, report: &WriteReport, completed_utc: String) -> Self {
        Self {
            draft: WriteDraft {
                request: draft.request.clone(),
                class3_size: draft.class3_size,
                expected_current: None,
                readback: false,
            },
            original: OriginalWrite {
                outcome: (&report.outcome).into(),
                expected_identity: report.expected_identity,
            },
            completed_utc: Some(completed_utc),
        }
    }

    /// Validate a completed GUI or CLI write receipt without connecting to the recorded endpoint.
    pub fn load(path: &Path) -> Result<Self, String> {
        read_receipt(path, Self::parse_original)
    }

    fn parse_original(bytes: &[u8]) -> Result<Self, String> {
        let (draft, completed_utc, original) = decode(bytes)?;
        Ok(Self {
            draft,
            original,
            completed_utc,
        })
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

    /// Describe a later check alongside the reference to the original write.
    pub fn follow_up_document(&self, check: &WriteCurrentCheck) -> Value {
        let mut report = self.report_document();
        report["result"] = json!(check);
        report
    }

    /// Check the original device identity before reading and comparing its current bytes.
    pub fn check_current(&self) -> Result<WriteCurrentCheck, String> {
        self.draft.check_current(self.original.expected_identity)
    }
}

impl SavedWriteRecord {
    /// Reopen a receipt or a completed follow-up without connecting to a device.
    pub fn load(path: &Path) -> Result<Self, String> {
        read_receipt(path, |bytes| {
            let header: RecordHeader =
                serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            match header.original_write {
                Some(original) => {
                    let (draft, completed_utc, check) = decode(bytes)?;
                    validate_saved_check(&draft, &original, &check)?;
                    Ok(Self {
                        receipt: SavedWriteReceipt {
                            draft,
                            original,
                            completed_utc,
                        },
                        check: Some(check),
                    })
                }
                None => Ok(Self {
                    receipt: SavedWriteReceipt::parse_original(bytes)?,
                    check: None,
                }),
            }
        })
    }

    pub fn receipt(&self) -> &SavedWriteReceipt {
        &self.receipt
    }

    pub fn recorded_check(&self) -> Option<&WriteCurrentCheck> {
        self.check.as_ref()
    }

    /// Describe the original write and any saved follow-up for offline review.
    pub fn report_document(&self) -> Value {
        if let Some(check) = &self.check {
            self.receipt.follow_up_document(check)
        } else {
            let mut report = self.receipt.report_document();
            report["result"] = json!({"outcome": "preview"});
            report
        }
    }
}

fn read_receipt<T>(
    path: &Path,
    parse: impl FnOnce(&[u8]) -> Result<T, String>,
) -> Result<T, String> {
    let load = || -> Result<T, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|file| file.take(MAX_RECEIPT_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_RECEIPT_BYTES {
            return Err("Write receipt exceeds 4 MiB".into());
        }
        parse(&bytes)
    };
    load().map_err(|error| format!("Load write receipt {} · {error}", path.display()))
}

fn decode<R: DeserializeOwned>(bytes: &[u8]) -> Result<(WriteDraft, Option<String>, R), String> {
    let doc: Document<R> = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if doc.service != "Set_Attribute_Single" {
        return Err("Expected a Set_Attribute_Single receipt".into());
    }
    let value = parse_hex(&doc.value_hex).map_err(|error| format!("Value bytes · {error}"))?;
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
            path: doc.path,
            route: doc.route,
            value,
        },
        class3_size: doc.class3_size,
        expected_current: None,
        readback: false,
    };
    draft.validate()?;
    Ok((draft, doc.completed_utc, doc.result))
}

fn validate_saved_check(
    draft: &WriteDraft,
    original: &OriginalWrite,
    check: &WriteCurrentCheck,
) -> Result<(), String> {
    let time = chrono::DateTime::parse_from_rfc3339(&check.checked_utc)
        .map_err(|error| format!("Check time · {error}"))?;
    if time.offset().local_minus_utc() != 0 {
        return Err("Check time must use UTC".into());
    }
    let identity_matches = original.expected_identity == check.observed_identity.pin();
    if original.expected_identity != check.expected_identity
        || check.identity_matches != identity_matches
    {
        return Err("Recorded device identity and check outcome disagree".into());
    }
    let skipped = matches!(check.comparison, ReadbackOutcome::Skipped);
    if skipped == identity_matches {
        return Err("Recorded comparison disagrees with the identity check".into());
    }
    if let Some(data) = check.comparison.data_hex() {
        let current = parse_hex(data).map_err(|error| format!("Checked value bytes · {error}"))?;
        let matched = matches!(check.comparison, ReadbackOutcome::Matched { .. });
        if (current == draft.request.value) != matched {
            return Err("Recorded comparison disagrees with the checked bytes".into());
        }
    }
    if draft.class3_size.is_some() != check.forward_close.is_some() {
        return Err("Recorded close outcome disagrees with the transport".into());
    }
    Ok(())
}
