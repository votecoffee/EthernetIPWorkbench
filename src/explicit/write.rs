use super::{
    Class3Size, EipError, Identity, IdentityPin, ReadPath, Route, RoutedReadError, Session,
    cip_data,
    connected::{connection_path, with_connection},
    identity, identity_from, is_unicast, read_message, reply_body,
};
use crate::{model::hex, scanner::CloseOutcome};
use serde::{Deserialize, Serialize};
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

#[derive(Debug)]
pub struct WriteInspection {
    pub identity: Identity,
    pub current: Result<Vec<u8>, RoutedReadError>,
    pub forward_close: Option<CloseOutcome>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WriteCurrentCheck {
    pub checked_utc: String,
    pub expected_identity: IdentityPin,
    pub observed_identity: Identity,
    pub identity_matches: bool,
    pub comparison: ReadbackOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_close: Option<CloseOutcome>,
}

impl WriteCurrentCheck {
    pub fn is_success(&self) -> bool {
        self.identity_matches
            && self.comparison.is_match()
            && self
                .forward_close
                .as_ref()
                .is_none_or(CloseOutcome::is_confirmed)
    }
}

struct InspectedValue {
    identity: Identity,
    current: Option<Result<Vec<u8>, RoutedReadError>>,
    forward_close: Option<CloseOutcome>,
}

impl WriteInspection {
    pub fn is_success(&self) -> bool {
        self.current.is_ok()
            && self
                .forward_close
                .as_ref()
                .is_none_or(CloseOutcome::is_confirmed)
    }
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
                cip_status(f, source, *general_status, extended_status)
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ReadbackOutcome {
    Skipped,
    Matched {
        data_hex: String,
    },
    Mismatch {
        data_hex: String,
    },
    Rejected {
        general_status: u8,
        extended_status: Vec<u16>,
    },
    RouterError {
        general_status: u8,
        extended_status: Vec<u16>,
    },
    Unavailable {
        error: String,
    },
}

impl fmt::Display for ReadbackOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.describe(
            f,
            "Readback",
            "the staged bytes",
            "target acknowledgment required",
        )
    }
}

impl ReadbackOutcome {
    fn is_match(&self) -> bool {
        matches!(self, Self::Matched { .. })
    }

    pub fn data_hex(&self) -> Option<&str> {
        match self {
            Self::Matched { data_hex } | Self::Mismatch { data_hex } => Some(data_hex),
            _ => None,
        }
    }

    fn describe(
        &self,
        f: &mut fmt::Formatter<'_>,
        check: &str,
        reference: &str,
        skipped_reason: &str,
    ) -> fmt::Result {
        match self {
            Self::Skipped => write!(f, "{check} skipped · {skipped_reason}"),
            Self::Matched { .. } => write!(f, "{check} matched {reference}"),
            Self::Mismatch { .. } => write!(f, "{check} differs from {reference}"),
            Self::Rejected {
                general_status,
                extended_status,
            } => cip_status(
                f,
                &format!("{check} rejected by target"),
                *general_status,
                extended_status,
            ),
            Self::RouterError {
                general_status,
                extended_status,
            } => cip_status(
                f,
                &format!("{check} rejected by router"),
                *general_status,
                extended_status,
            ),
            Self::Unavailable { error } => write!(f, "{check} unavailable · {error}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions<'a> {
    /// When present, require an exact read of these bytes before sending.
    pub expected_current: Option<&'a [u8]>,
    pub readback: bool,
}

#[derive(Debug, Serialize)]
pub struct CurrentValueCheck {
    pub expected_hex: String,
    #[serde(flatten)]
    pub outcome: ReadbackOutcome,
}

impl fmt::Display for CurrentValueCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.outcome.describe(
            f,
            "Current value",
            "the expected bytes",
            "verified target identity required",
        )
    }
}

fn cip_status(
    f: &mut fmt::Formatter<'_>,
    source: &str,
    general_status: u8,
    extended_status: &[u16],
) -> fmt::Result {
    write!(f, "{source} · CIP general 0x{general_status:02X}")?;
    for word in extended_status {
        write!(f, ", extended 0x{word:04X}")?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct WriteReport {
    pub expected_identity: IdentityPin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_identity: Option<Identity>,
    #[serde(flatten)]
    pub outcome: WriteOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_value: Option<CurrentValueCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readback: Option<ReadbackOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_close: Option<CloseOutcome>,
}

impl WriteReport {
    fn pending(expected_identity: IdentityPin, options: WriteOptions<'_>) -> Self {
        Self {
            expected_identity,
            observed_identity: None,
            outcome: WriteOutcome::NotSent {
                error: "Write review did not complete".into(),
            },
            current_value: options.expected_current.map(|bytes| CurrentValueCheck {
                expected_hex: hex(bytes),
                outcome: ReadbackOutcome::Skipped,
            }),
            readback: options.readback.then_some(ReadbackOutcome::Skipped),
            forward_close: None,
        }
    }

    /// True when the write, requested readback, and any Forward_Close all succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self.outcome, WriteOutcome::Acknowledged { .. })
            && self.readback.as_ref().is_none_or(ReadbackOutcome::is_match)
            && self
                .forward_close
                .as_ref()
                .is_none_or(CloseOutcome::is_confirmed)
    }
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
        let mut session = self.open_session()?;
        identity(&mut session, self.route.as_ref())
            .map_err(|error| format!("Read target identity · {error}"))
    }

    pub fn validate_connected(&self, size: Class3Size) -> Result<(), String> {
        self.connected_message(size).map(|_| ())
    }

    fn connected_message(&self, size: Class3Size) -> Result<Vec<u8>, String> {
        let request = self.message()?;
        connection_path(self.route.as_ref())?;
        size.check_request(request.len(), "write")?;
        Ok(request)
    }

    /// Read the target's identity over Class 3 for review, then close the connection.
    pub fn read_target_identity_connected(&self, size: Class3Size) -> Result<Identity, String> {
        self.validate_connected(size)?;
        let (observed, close) = with_connection(
            self.adapter_or_bridge,
            self.local_ip,
            self.route.as_ref(),
            size,
            |connection| identity_from(|path| connection.request(&read_message(path))),
        )
        .map_err(|error| error.to_string())?;
        if !close.is_confirmed() {
            return Err(format!("Class 3 identity review · Forward_Close {close}"));
        }
        observed.map_err(|error| format!("Read target identity · {error}"))
    }

    pub(super) fn inspect_current(
        &self,
        size: Option<Class3Size>,
    ) -> Result<WriteInspection, String> {
        let inspected = self.inspect(size, None)?;
        Ok(WriteInspection {
            identity: inspected.identity,
            current: inspected.current.ok_or("Attribute was not read")?,
            forward_close: inspected.forward_close,
        })
    }

    pub(super) fn check_current(
        &self,
        size: Option<Class3Size>,
        expected_identity: IdentityPin,
    ) -> Result<WriteCurrentCheck, String> {
        let inspected = self.inspect(size, Some(expected_identity))?;
        let comparison = inspected
            .current
            .map_or(ReadbackOutcome::Skipped, |current| {
                compare_value(current, &self.value)
            });
        Ok(WriteCurrentCheck {
            checked_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            expected_identity,
            identity_matches: inspected.identity.pin() == expected_identity,
            observed_identity: inspected.identity,
            comparison,
            forward_close: inspected.forward_close,
        })
    }

    fn inspect(
        &self,
        size: Option<Class3Size>,
        expected_identity: Option<IdentityPin>,
    ) -> Result<InspectedValue, String> {
        let identity_error = |error| format!("Read target identity · {error}");
        let same_target =
            |actual: &Identity| expected_identity.is_none_or(|pin| actual.pin() == pin);
        if let Some(size) = size {
            self.validate_connected(size)?;
            let (observed, close) = with_connection(
                self.adapter_or_bridge,
                self.local_ip,
                self.route.as_ref(),
                size,
                |connection| {
                    let identity = identity_from(|path| connection.request(&read_message(path)))?;
                    let value = same_target(&identity).then(|| {
                        connection
                            .request(&read_message(self.path))
                            .map_err(target_error)
                    });
                    Ok::<_, EipError>((identity, value))
                },
            )
            .map_err(|error| error.to_string())?;
            let (identity, current) = observed
                .map_err(|error| format!("{}. Forward_Close {close}", identity_error(error)))?;
            return Ok(InspectedValue {
                identity,
                current,
                forward_close: Some(close),
            });
        }
        self.validate()?;
        let mut session = self.open_session()?;
        let identity = identity(&mut session, self.route.as_ref()).map_err(identity_error)?;
        let current = same_target(&identity).then(|| match &self.route {
            Some(route) => session.read_routed(self.path, route),
            None => session.read(self.path).map_err(target_error),
        });
        Ok(InspectedValue {
            identity,
            current,
            forward_close: None,
        })
    }

    /// Check the pinned identity and send the reviewed write once on that TCP session.
    /// A router reply or loss of the target reply leaves application of the value unconfirmed.
    pub fn apply(&self, expected_identity: IdentityPin) -> Result<WriteReport, String> {
        self.apply_with_options(expected_identity, WriteOptions::default())
    }

    /// Read the same attribute on the write session after a target acknowledgment.
    /// Preserve the acknowledgment even when the read fails or returns different bytes.
    pub fn apply_with_readback(
        &self,
        expected_identity: IdentityPin,
    ) -> Result<WriteReport, String> {
        self.apply_with_options(
            expected_identity,
            WriteOptions {
                readback: true,
                ..WriteOptions::default()
            },
        )
    }

    /// Apply a reviewed write with optional checks on the same session.
    /// The current-value check is a separate read immediately before the write.
    pub fn apply_with_options(
        &self,
        expected_identity: IdentityPin,
        options: WriteOptions<'_>,
    ) -> Result<WriteReport, String> {
        let request = self.message()?;
        let mut report = WriteReport::pending(expected_identity, options);
        match self.open_session() {
            Ok(mut session) => {
                let actual = identity(&mut session, self.route.as_ref());
                self.run_reviewed(&request, actual, options, &mut report, |cip| {
                    let decode = |data: &[u8]| Ok(reply_body(data, cip[0])?.to_vec());
                    match &self.route {
                        Some(route) => session.routed(cip, route, decode),
                        None => session
                            .unconnected(cip.to_vec(), None)
                            .and_then(|reply| decode(cip_data(&reply)?))
                            .map_err(target_error),
                    }
                });
            }
            Err(error) => report.outcome = WriteOutcome::NotSent { error },
        }
        Ok(report)
    }

    /// Apply a reviewed write and optional checks on one Class 3 connection.
    /// A failed or missing write reply does not trigger a retry.
    pub fn apply_connected(
        &self,
        expected_identity: IdentityPin,
        options: WriteOptions<'_>,
        size: Class3Size,
    ) -> Result<WriteReport, String> {
        let request = self.connected_message(size)?;
        let mut report = WriteReport::pending(expected_identity, options);
        let result = with_connection(
            self.adapter_or_bridge,
            self.local_ip,
            self.route.as_ref(),
            size,
            |connection| {
                let actual = identity_from(|path| connection.request(&read_message(path)));
                self.run_reviewed(&request, actual, options, &mut report, |cip| {
                    connection.request(cip).map_err(target_error)
                });
            },
        );
        match result {
            Ok(((), close)) => report.forward_close = Some(close),
            Err(error) => {
                report.outcome = WriteOutcome::NotSent {
                    error: error.message,
                };
                report.forward_close = error.forward_close;
            }
        }
        Ok(report)
    }

    fn open_session(&self) -> Result<Session, String> {
        Session::open(self.adapter_or_bridge, self.local_ip)
            .map_err(|error| format!("RegisterSession · {error}"))
    }

    fn run_reviewed(
        &self,
        request: &[u8],
        actual: Result<Identity, EipError>,
        options: WriteOptions<'_>,
        report: &mut WriteReport,
        mut exchange: impl FnMut(&[u8]) -> Result<Vec<u8>, RoutedReadError>,
    ) {
        let actual = match actual {
            Ok(actual) => actual,
            Err(error) => {
                report.outcome = WriteOutcome::NotSent {
                    error: format!("Read target identity · {error}"),
                };
                return;
            }
        };
        let identity_matches = actual.pin() == report.expected_identity;
        report.observed_identity = Some(actual);
        if !identity_matches {
            report.outcome = WriteOutcome::NotSent {
                error: "Target identity did not match the reviewed device".into(),
            };
            return;
        }
        let read = read_message(self.path);
        if let (Some(expected), Some(check)) = (options.expected_current, &mut report.current_value)
        {
            check.outcome = compare_value(exchange(&read), expected);
            if !check.outcome.is_match() {
                report.outcome = WriteOutcome::NotSent {
                    error: "Current value check did not pass".into(),
                };
                return;
            }
        }
        report.outcome = write_outcome(exchange(request));
        if options.readback && matches!(report.outcome, WriteOutcome::Acknowledged { .. }) {
            report.readback = Some(compare_value(exchange(&read), &self.value));
        }
    }
}

fn target_error(error: EipError) -> RoutedReadError {
    match error {
        EipError::Cip { .. } => RoutedReadError::Target(error),
        _ => RoutedReadError::Exchange(error),
    }
}

fn write_outcome(result: Result<Vec<u8>, RoutedReadError>) -> WriteOutcome {
    match result {
        Ok(data) => WriteOutcome::Acknowledged {
            reply_hex: hex(&data),
        },
        Err(RoutedReadError::Target(EipError::Cip { status, ext })) => WriteOutcome::Rejected {
            general_status: status,
            extended_status: ext,
        },
        Err(RoutedReadError::Router(EipError::Cip { status, ext })) => WriteOutcome::RouterError {
            general_status: status,
            extended_status: ext,
        },
        Err(RoutedReadError::Exchange(error)) => WriteOutcome::Unconfirmed {
            error: error.to_string(),
        },
        Err(error) => WriteOutcome::Unconfirmed {
            error: error.to_string(),
        },
    }
}

fn compare_value(result: Result<Vec<u8>, RoutedReadError>, expected: &[u8]) -> ReadbackOutcome {
    match result {
        Ok(data) if data == expected => ReadbackOutcome::Matched {
            data_hex: hex(&data),
        },
        Ok(data) => ReadbackOutcome::Mismatch {
            data_hex: hex(&data),
        },
        Err(RoutedReadError::Target(EipError::Cip { status, ext })) => ReadbackOutcome::Rejected {
            general_status: status,
            extended_status: ext,
        },
        Err(RoutedReadError::Router(EipError::Cip { status, ext })) => {
            ReadbackOutcome::RouterError {
                general_status: status,
                extended_status: ext,
            }
        }
        Err(RoutedReadError::Exchange(error)) => ReadbackOutcome::Unavailable {
            error: error.to_string(),
        },
        Err(error) => ReadbackOutcome::Unavailable {
            error: error.to_string(),
        },
    }
}
