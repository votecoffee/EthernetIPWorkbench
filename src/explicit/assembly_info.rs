//! Read-only inspection of one CIP Assembly Object instance.

use super::{Route, SavedReadBatch, SavedReadOutcome, batch::read_object_attributes};
use std::net::Ipv4Addr;

pub const ASSEMBLY_CLASS: u32 = 0x04;
const DATA_ATTRIBUTE: u32 = 3;
pub(super) const SIZE_ATTRIBUTE: u32 = 4;
const ATTRIBUTES: &[(u32, &str)] = &[
    (SIZE_ATTRIBUTE, "Declared size"),
    (DATA_ATTRIBUTE, "Assembly data"),
];

pub struct AssemblyInfo {
    pub instance: u32,
    pub route: Option<Route>,
    pub batch: SavedReadBatch,
}

impl AssemblyInfo {
    /// True after a usable batch. Unsupported target attributes may be absent.
    pub fn is_usable(&self) -> bool {
        self.batch
            .is_usable_inspection_with_raw_attribute(Some(DATA_ATTRIBUTE))
    }

    /// Assembly byte count from attribute 4, when available and correctly encoded.
    pub fn declared_size(&self) -> Option<u16> {
        self.reply(SIZE_ATTRIBUTE)
            .and_then(|bytes| size(bytes).ok())
    }

    /// Number of payload bytes in the separate attribute 3 reply.
    pub fn data_size(&self) -> Option<usize> {
        self.reply(DATA_ATTRIBUTE).map(<[u8]>::len)
    }

    /// Compare the explicit reply lengths when both attributes are available.
    pub fn sizes_match(&self) -> Option<bool> {
        Some(usize::from(self.declared_size()?) == self.data_size()?)
    }

    fn reply(&self, attribute: u32) -> Option<&[u8]> {
        self.batch
            .results
            .iter()
            .find_map(|entry| match &entry.outcome {
                SavedReadOutcome::Reply(bytes)
                    if entry.request.path.attribute == Some(attribute) =>
                {
                    Some(bytes.as_slice())
                }
                _ => None,
            })
    }
}

/// Read the declared size and data as separate attributes on one explicit session.
pub fn read_assembly_info(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<AssemblyInfo, String> {
    if instance == 0 {
        return Err("Assembly inspection requires a nonzero instance".into());
    }
    let batch = read_object_attributes(
        adapter,
        local,
        ASSEMBLY_CLASS,
        instance,
        route.as_ref(),
        ATTRIBUTES,
    )?;
    Ok(AssemblyInfo {
        instance,
        route,
        batch,
    })
}

pub(super) fn decode(attribute: u32, bytes: &[u8]) -> Option<Result<String, String>> {
    match attribute {
        SIZE_ATTRIBUTE => Some(size(bytes).map(|size| format!("{size} bytes"))),
        _ => None,
    }
}

pub(super) fn size(bytes: &[u8]) -> Result<u16, String> {
    bytes.try_into().map(u16::from_le_bytes).map_err(|_| {
        format!(
            "Assembly size requires a two-byte UINT, got {} bytes",
            bytes.len()
        )
    })
}
