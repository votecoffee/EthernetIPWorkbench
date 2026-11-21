//! Read-only diagnostics for a single active CIP connection on a supporting target.

use super::{
    CONNECTION_MANAGER_CLASS, EipError, ReadPath, Reader, Route, RoutedReadError, encode_read_path,
    le16, le32, parse_id,
};
use serde::Serialize;
use std::{fmt, net::Ipv4Addr, str::FromStr};

pub(super) const PATH: ReadPath = ReadPath {
    class: CONNECTION_MANAGER_CLASS,
    instance: 1,
    attribute: None,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ConnectionTriad {
    pub connection_serial: u16,
    pub originator_vendor: u16,
    pub originator_serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum ConnectionDataQuery {
    Number { number: u16 },
    Triad(ConnectionTriad),
}

impl FromStr for ConnectionDataQuery {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let parse_word = |field, name| {
            u16::try_from(parse_id(field)?).map_err(|_| format!("{name} must fit in 16 bits"))
        };
        if !input.contains(':') {
            return Ok(Self::Number {
                number: parse_word(input, "Connection number")?,
            });
        }
        let parts: Vec<_> = input.splitn(4, ':').collect();
        let [serial, vendor, originator] = parts.as_slice() else {
            return Err(
                "Enter connection number or connection-serial:vendor:originator-serial".into(),
            );
        };
        Ok(Self::Triad(ConnectionTriad {
            connection_serial: parse_word(serial, "Connection serial")?,
            originator_vendor: parse_word(vendor, "Originator vendor")?,
            originator_serial: parse_id(originator)?,
        }))
    }
}

impl fmt::Display for ConnectionDataQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number { number } => write!(f, "Connection number {number}"),
            Self::Triad(triad) => write!(
                f,
                "Connection serial 0x{:04X} · Vendor {} · Originator serial 0x{:08X}",
                triad.connection_serial, triad.originator_vendor, triad.originator_serial,
            ),
        }
    }
}

impl ConnectionDataQuery {
    pub fn service_name(self) -> &'static str {
        match self {
            Self::Number { .. } => "Get_Connection_Data",
            Self::Triad(_) => "Search_Connection_Data",
        }
    }

    fn request(self) -> Vec<u8> {
        match self {
            Self::Number { number } => {
                let mut request = encode_read_path(0x56, PATH);
                request.extend_from_slice(&number.to_le_bytes());
                request
            }
            Self::Triad(triad) => {
                let mut request = encode_read_path(0x57, PATH);
                request.extend_from_slice(&triad.connection_serial.to_le_bytes());
                request.extend_from_slice(&triad.originator_vendor.to_le_bytes());
                request.extend_from_slice(&triad.originator_serial.to_le_bytes());
                request
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectionDirection {
    pub originator_connection_id: u32,
    pub target_connection_id: u32,
    pub timeout_multiplier_code: u8,
    pub reported_rpi_us: u32,
    pub reported_api_us: u32,
}

impl ConnectionDirection {
    fn decode(data: &[u8; 20]) -> Self {
        Self {
            originator_connection_id: le32(data, 0),
            target_connection_id: le32(data, 4),
            timeout_multiplier_code: data[8],
            reported_rpi_us: le32(data, 12),
            reported_api_us: le32(data, 16),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectionData {
    pub number: u16,
    pub state_code: u16,
    pub originator_port: u16,
    pub target_port: u16,
    pub triad: ConnectionTriad,
    pub o_to_t: ConnectionDirection,
    pub t_to_o: ConnectionDirection,
}

pub struct ConnectionDataReport {
    pub adapter_or_bridge: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub route: Option<Route>,
    pub query: ConnectionDataQuery,
    pub reply: Result<Vec<u8>, RoutedReadError>,
}

impl ConnectionDataReport {
    pub fn decoded(&self) -> Result<Option<ConnectionData>, String> {
        self.reply
            .as_ref()
            .ok()
            .map(|bytes| decode(bytes, self.query))
            .transpose()
    }
}

/// Query one Connection Manager record by its number or the triad from Forward_Open.
pub fn read_connection_data(
    adapter: Ipv4Addr,
    local: Ipv4Addr,
    query: ConnectionDataQuery,
    route: Option<Route>,
) -> Result<ConnectionDataReport, String> {
    let mut reader = Reader::connect(adapter, local).map_err(|error| error.to_string())?;
    let session = reader
        .0
        .as_mut()
        .ok_or_else(|| EipError::Closed.to_string())?;
    let reply = session.read_service(query.request(), route.as_ref());
    Ok(ConnectionDataReport {
        adapter_or_bridge: adapter,
        local_ip: local,
        route,
        query,
        reply,
    })
}

fn decode(bytes: &[u8], query: ConnectionDataQuery) -> Result<ConnectionData, String> {
    let bytes: &[u8; 56] = bytes
        .try_into()
        .map_err(|_| format!("Connection data requires 56 bytes, got {}", bytes.len()))?;
    let number = le16(bytes, 0);
    let triad = ConnectionTriad {
        connection_serial: le16(bytes, 8),
        originator_vendor: le16(bytes, 10),
        originator_serial: le32(bytes, 12),
    };
    let matches = match query {
        ConnectionDataQuery::Number { number: requested } => requested == number,
        ConnectionDataQuery::Triad(requested) => requested == triad,
    };
    if !matches {
        return Err("Returned connection identifiers differ from the requested connection".into());
    }
    Ok(ConnectionData {
        number,
        state_code: le16(bytes, 2),
        originator_port: le16(bytes, 4),
        target_port: le16(bytes, 6),
        triad,
        o_to_t: ConnectionDirection::decode(bytes[16..36].try_into().unwrap()),
        t_to_o: ConnectionDirection::decode(bytes[36..56].try_into().unwrap()),
    })
}
