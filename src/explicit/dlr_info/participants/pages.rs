//! Read a roster in bounded Get_Member pages on the count reads' TCP session.

use super::{
    ATTRIBUTES, DLR_CLASS, DlrParticipantReport, OVERFLOW_MESSAGE, RingRecords, records, uint,
};
use crate::explicit::{
    ReadPath, Reader, Route, RoutedReadError, SavedRead, SavedReadBatch, SavedReadOutcome,
    SavedReadResult, encode_read_path, logical_segment,
};
use std::net::Ipv4Addr;

const GET_MEMBER: u8 = 0x18;
const MULTIPLE_SEQUENTIAL_MEMBERS: u8 = 1;
const MEMBER_EXTENDED_PROTOCOL: u16 = 0x8000;
const PAGE_SIZE: u16 = 32;
const MAX_PAGES: usize = 32;

pub struct DlrParticipantPage {
    pub first_member: u16,
    pub requested_count: u16,
    pub reply: Result<Vec<u8>, RoutedReadError>,
}

impl DlrParticipantPage {
    pub fn returned_count(&self) -> Result<Option<usize>, String> {
        self.decoded()
            .map(|page| page.map(|page| page.addresses.len()))
    }

    fn decoded(&self) -> Result<Option<RingRecords<'_>>, String> {
        let Ok(bytes) = &self.reply else {
            return Ok(None);
        };
        let (count, body) = bytes
            .split_first_chunk::<2>()
            .ok_or("Get_Member reply is missing its member count")?;
        let count = u16::from_le_bytes(*count);
        if count == 0 || count > self.requested_count {
            return Err(format!(
                "Get_Member returned count {count} for {} requested members",
                self.requested_count
            ));
        }
        let (first, data) = body
            .split_first_chunk::<2>()
            .ok_or("Get_Member reply is missing its first member ID")?;
        let first = u16::from_le_bytes(*first);
        if first != self.first_member {
            return Err(format!(
                "Get_Member replied with first member {first}, expected {}",
                self.first_member
            ));
        }
        if data.len() != usize::from(count) * 10 {
            return Err(format!(
                "Get_Member count {count} does not match {} record bytes",
                data.len()
            ));
        }
        records(data).map(Some)
    }
}

/// Read up to 32 pages of 32 consecutive ring participants. A failed page
/// stops paging and preserves any prior entries and the original reply.
pub fn read_dlr_participants_paged(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    instance: u32,
    route: Option<Route>,
) -> Result<DlrParticipantReport, String> {
    if instance == 0 {
        return Err("DLR participant inspection requires a nonzero instance".into());
    }
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let before = count_read(&mut reader, instance, route.as_ref(), ATTRIBUTES[0], true);
    let count_before = count_value(&before.outcome);
    let mut session_available = !matches!(before.outcome, SavedReadOutcome::ExchangeFailed(_));
    let mut list_issue = None;
    let mut pages = Vec::new();
    let mut participants = None;
    if let Some(count) = count_before {
        if count == 0 {
            participants = Some(Vec::new());
        }
        let mut remaining = count;
        for _ in 0..MAX_PAGES {
            if remaining == 0 {
                break;
            }
            let first_member = count - remaining + 1;
            let requested_count = remaining.min(PAGE_SIZE);
            let page = DlrParticipantPage {
                first_member,
                requested_count,
                reply: read_page(
                    &mut reader,
                    instance,
                    route.as_ref(),
                    first_member,
                    requested_count,
                ),
            };
            session_available = !matches!(page.reply, Err(RoutedReadError::Exchange(_)));
            match page.decoded() {
                Ok(Some(decoded)) => {
                    remaining -= decoded.addresses.len() as u16;
                    participants
                        .get_or_insert_with(Vec::new)
                        .extend(decoded.nodes());
                    if decoded.overflow {
                        list_issue = Some(OVERFLOW_MESSAGE.into());
                    }
                }
                Ok(None) => {
                    list_issue = Some(format!(
                        "Could not read the page starting at member {first_member}."
                    ))
                }
                Err(error) => list_issue = Some(error),
            }
            pages.push(page);
            if list_issue.is_some() {
                break;
            }
        }
        if remaining > 0 && list_issue.is_none() {
            list_issue = Some(format!("Stopped after the {MAX_PAGES}-page read limit."));
        }
    } else {
        list_issue = Some("The initial participant count was unavailable.".into());
    }
    let after = count_read(
        &mut reader,
        instance,
        route.as_ref(),
        ATTRIBUTES[2],
        session_available,
    );
    let count_after = count_value(&after.outcome);
    Ok(DlrParticipantReport {
        instance,
        route,
        batch: SavedReadBatch {
            adapter_or_bridge: adapter,
            local_ip: local,
            results: vec![before, after],
        },
        count_before,
        participants,
        count_after,
        pages: Some(pages),
        list_issue,
    })
}

fn count_read(
    reader: &mut Reader,
    instance: u32,
    route: Option<&Route>,
    (attribute, name): (u32, &str),
    session_available: bool,
) -> SavedReadResult {
    let path = ReadPath {
        class: DLR_CLASS,
        instance,
        attribute: Some(attribute),
    };
    let outcome = if session_available {
        SavedReadOutcome::from(reader.read_request(path, route))
    } else {
        SavedReadOutcome::NotAttempted
    };
    SavedReadResult {
        request: SavedRead {
            path,
            route: route.cloned(),
            name: name.into(),
            selection: None,
        },
        outcome,
    }
}

fn count_value(outcome: &SavedReadOutcome) -> Option<u16> {
    match outcome {
        SavedReadOutcome::Reply(bytes) => uint(bytes).ok(),
        _ => None,
    }
}

fn read_page(
    reader: &mut Reader,
    instance: u32,
    route: Option<&Route>,
    first: u16,
    count: u16,
) -> Result<Vec<u8>, RoutedReadError> {
    let mut request = encode_read_path(
        GET_MEMBER,
        ReadPath {
            class: DLR_CLASS,
            instance,
            attribute: Some(9),
        },
    );
    // Use a 16-bit Member ID so bit 15 selects the extended member protocol.
    logical_segment(
        &mut request,
        0x28,
        u32::from(MEMBER_EXTENDED_PROTOCOL | first),
    );
    request[1] = ((request.len() - 2) / 2) as u8;
    request.push(MULTIPLE_SEQUENTIAL_MEMBERS);
    request.extend(count.to_le_bytes());
    reader.read_service(request, route)
}
