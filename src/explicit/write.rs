use super::{
    EipError, Identity, IdentityPin, ReadPath, Route, RoutedReadError, Session, cip_data, identity,
    is_unicast, read_message, reply_body,
};
use crate::model::hex;
use serde::Serialize;
use std::{fmt, net::Ipv4Addr};

const SET_ATTRIBUTE_SINGLE: u8 = 0x10;
// Eight bytes of SendRRData fields and two four-byte CPF item headers.
const MAX_MESSAGE: usize = u16::MAX as usize - 16;

/// A single explicit attribute write. The value uses the target's documented wire encoding.
#[derive(Clone, Debug)]
pub struct AttributeWrite {
    pub adapter_or_bridge: Ipv4Addr,
    pub local_ip: Ipv4Addr,
    pub path: ReadPath,
    pub route: Option<Route>,
    pub value: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WriteOutcome {
    NotSent {
        error: String,
    },
    Acknowledged {
        reply_hex: String,
    },
    Rejected {
        general_status: u8,
        extended_status: Vec<u16>,
    },
    RouterError {
        general_status: u8,
        extended_status: Vec<u16>,
    },
    Unconfirmed {
        error: String,
    },
}

impl fmt::Display for WriteOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSent { error } => write!(f, "Write not sent · {error}"),
            Self::Acknowledged { .. } => f.write_str("Write acknowledged by target"),
            Self::Unconfirmed { error } => write!(f, "Write outcome unconfirmed · {error}"),
            Self::Rejected {
                general_status,
                extended_status,
            }
            | Self::RouterError {
                general_status,
                extended_status,
            } => {
                let source = if matches!(self, Self::Rejected { .. }) {
                    "Target rejected write"
                } else {
                    "Router replied · target outcome unconfirmed"
                };
                write!(f, "{source} · CIP general 0x{general_status:02X}")?;
                for word in extended_status {
                    write!(f, ", extended 0x{word:04X}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WriteReport {
    pub expected_identity: IdentityPin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_identity: Option<Identity>,
    #[serde(flatten)]
    pub outcome: WriteOutcome,
}

impl AttributeWrite {
    fn message(&self) -> Result<Vec<u8>, String> {
        if !is_unicast(self.adapter_or_bridge) || !is_unicast(self.local_ip) {
            return Err("Enter unicast adapter and local NIC IPv4 addresses".into());
        }
        self.path.validate()?;
        if self.path.attribute.is_none() {
            return Err("Enter a single attribute ID for Set_Attribute_Single".into());
        }
        let mut message = read_message(self.path);
        message[0] = SET_ATTRIBUTE_SINGLE;
        let size = message.len().saturating_add(self.value.len());
        let extra = self
            .route
            .as_ref()
            .map_or(0, |route| 12 + (size % 2) + route.port_segments().len());
        if size.saturating_add(extra) > MAX_MESSAGE {
            return Err("Attribute write exceeds the EtherNet/IP encapsulation length".into());
        }
        message.extend(&self.value);
        Ok(message)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.message().map(|_| ())
    }

    /// Read the proposed target's identity without sending the staged value.
    pub fn read_target_identity(&self) -> Result<Identity, String> {
        self.validate()?;
        let mut session = Session::open(self.adapter_or_bridge, self.local_ip)
            .map_err(|error| error.to_string())?;
        identity(&mut session, self.route.as_ref()).map_err(|error| error.to_string())
    }

    /// Check the pinned identity and send the reviewed write once on that TCP session.
    /// A router reply or loss of the target reply leaves application of the value unconfirmed.
    pub fn apply(&self, expected_identity: IdentityPin) -> Result<WriteReport, String> {
        let request = self.message()?;
        let mut observed_identity = None;
        let outcome = self.write_once(request, expected_identity, &mut observed_identity);
        Ok(WriteReport {
            expected_identity,
            observed_identity,
            outcome,
        })
    }

    fn write_once(
        &self,
        request: Vec<u8>,
        expected: IdentityPin,
        observed: &mut Option<Identity>,
    ) -> WriteOutcome {
        let target = || {
            let mut session = Session::open(self.adapter_or_bridge, self.local_ip)
                .map_err(|error| format!("RegisterSession · {error}"))?;
            let actual = identity(&mut session, self.route.as_ref())
                .map_err(|error| format!("Read target identity · {error}"))?;
            Ok::<_, String>((session, actual))
        };
        let (mut session, actual) = match target() {
            Ok(target) => target,
            Err(error) => return WriteOutcome::NotSent { error },
        };
        let actual_pin = actual.pin();
        *observed = Some(actual);
        if actual_pin != expected {
            return WriteOutcome::NotSent {
                error: "Target identity did not match the reviewed device".into(),
            };
        }
        let decode = |data: &[u8]| Ok(reply_body(data, SET_ATTRIBUTE_SINGLE)?.to_vec());
        let result = match &self.route {
            Some(route) => session.routed(&request, route, decode),
            None => session
                .unconnected(request, None)
                .and_then(|reply| decode(cip_data(&reply)?))
                .map_err(|error| match error {
                    error @ EipError::Cip { .. } => RoutedReadError::Target(error),
                    error => RoutedReadError::Exchange(error),
                }),
        };
        match result {
            Ok(data) => WriteOutcome::Acknowledged {
                reply_hex: hex(&data),
            },
            Err(RoutedReadError::Target(EipError::Cip { status, ext })) => WriteOutcome::Rejected {
                general_status: status,
                extended_status: ext,
            },
            Err(RoutedReadError::Router(EipError::Cip { status, ext })) => {
                WriteOutcome::RouterError {
                    general_status: status,
                    extended_status: ext,
                }
            }
            Err(RoutedReadError::Exchange(error)) => WriteOutcome::Unconfirmed {
                error: error.to_string(),
            },
            Err(error) => WriteOutcome::Unconfirmed {
                error: error.to_string(),
            },
        }
    }
}
