//! CIP port paths and Unconnected Send read replies.

use super::{EipError, MAX_CIP_PATH_BYTES, is_unicast, protocol};
use serde::{Deserialize, Serialize};
use std::{fmt, net::Ipv4Addr, str::FromStr};

pub(super) const UNCONNECTED_SEND: u8 = 0x52;
const MAX_HOPS: usize = 8;

/// A validated list of port/link pairs, for example `1/3` for backplane slot 3.
/// Each port is numbered 1–65535. Links accept a decimal byte, a unicast IPv4
/// address, or 2–255 exact link bytes as contiguous hex after the hex: prefix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Route(Vec<Hop>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hop {
    port: u16,
    link: Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Link {
    Node(u8),
    Extended(Vec<u8>),
}

impl FromStr for Link {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if let Some(raw) = text.strip_prefix("hex:") {
            let length = raw.len() / 2;
            if !(2..=u8::MAX as usize).contains(&length)
                || raw.len() % 2 != 0
                || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(
                    "A hex CIP link requires 2 to 255 bytes as contiguous pairs of hex digits",
                );
            }
            let bytes = (0..raw.len())
                .step_by(2)
                .map(|index| {
                    u8::from_str_radix(&raw[index..index + 2], 16).expect("ASCII hex pair")
                })
                .collect();
            return Ok(Self::Extended(bytes));
        }
        if let Ok(node) = text.parse::<u8>() {
            return Ok(Self::Node(node));
        }
        text.parse::<Ipv4Addr>()
            .ok()
            .filter(|ip| is_unicast(*ip))
            .map(|ip| Self::Extended(ip.to_string().into_bytes()))
            .ok_or("CIP links need a node number from 0 to 255, a unicast IPv4 address, or hex link bytes")
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(node) => write!(f, "{node}"),
            Self::Extended(bytes) => {
                if let Ok(text) = std::str::from_utf8(bytes)
                    && text.parse::<Ipv4Addr>().is_ok_and(is_unicast)
                {
                    return f.write_str(text);
                }
                f.write_str("hex:")?;
                for byte in bytes {
                    write!(f, "{byte:02X}")?;
                }
                Ok(())
            }
        }
    }
}

impl FromStr for Route {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.trim().split('/');
        let mut hops = Vec::new();
        while let Some(port) = parts.next() {
            if hops.len() == MAX_HOPS {
                return Err(format!("CIP routes support at most {MAX_HOPS} hops"));
            }
            let port = port
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|&n| n != 0)
                .ok_or("Enter a CIP port number from 1 to 65535")?;
            let link = parts
                .next()
                .ok_or("A CIP route needs a link after each port")?
                .trim()
                .parse()?;
            hops.push(Hop { port, link });
        }
        let route = Self(hops);
        if route.port_segments().len() > MAX_CIP_PATH_BYTES {
            return Err("Encoded CIP route exceeds 255 words".into());
        }
        Ok(route)
    }
}

impl TryFrom<String> for Route {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Route> for String {
    fn from(route: Route) -> Self {
        route.to_string()
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, hop) in self.0.iter().enumerate() {
            if index != 0 {
                f.write_str("/")?;
            }
            write!(f, "{}/{}", hop.port, hop.link)?;
        }
        Ok(())
    }
}

impl Route {
    pub(super) fn from_port_segment(bytes: &[u8]) -> Result<Self, &'static str> {
        let (&header, mut rest) = bytes.split_first().ok_or("Port segment is empty")?;
        if header & 0xe0 != 0 {
            return Err("Expected a CIP port segment");
        }
        let mut length = 1;
        if header & 0x10 != 0 {
            let (&size, link) = rest
                .split_first()
                .ok_or("Port segment is missing its link length")?;
            length = usize::from(size);
            rest = link;
        }
        let mut port = u16::from(header & 0x0f);
        if port == 0x0f {
            let (number, link) = rest
                .split_first_chunk()
                .ok_or("Extended port number is truncated")?;
            port = u16::from_le_bytes(*number);
            rest = link;
        }
        if port == 0 || length == 0 {
            return Err("Port segment requires a nonzero port and link length");
        }
        let padding = (bytes.len() - rest.len() + length) % 2;
        if rest.len() != length + padding || (padding != 0 && rest[length] != 0) {
            return Err("Port segment length or padding does not match its link size");
        }
        let link = if length == 1 {
            Link::Node(rest[0])
        } else {
            Link::Extended(rest[..length].to_vec())
        };
        Ok(Self(vec![Hop { port, link }]))
    }

    pub(crate) fn port_segments(&self) -> Vec<u8> {
        let mut path = Vec::new();
        for hop in &self.0 {
            match &hop.link {
                Link::Node(node) => append_port_segment(&mut path, hop.port, &[*node]),
                Link::Extended(bytes) => append_port_segment(&mut path, hop.port, bytes),
            }
        }
        path
    }

    pub(super) fn wrap(&self, message: &[u8]) -> Vec<u8> {
        let path = self.port_segments();
        // Connection Manager instance 1. Eight 128 ms ticks leave time for
        // the routing rejection to arrive before the 2 s TCP reply deadline.
        let mut request = vec![UNCONNECTED_SEND, 2, 0x20, 6, 0x24, 1, 7, 8];
        request.extend((message.len() as u16).to_le_bytes());
        request.extend(message);
        if message.len() % 2 != 0 {
            request.push(0);
        }
        request.extend([(path.len() / 2) as u8, 0]);
        request.extend(path);
        request
    }
}

fn append_port_segment(path: &mut Vec<u8>, port: u16, link: &[u8]) {
    const EXTENDED_PORT: u16 = 0x0f;
    const EXTENDED_LINK: u8 = 0x10;

    let long_link = link.len() != 1;
    let flags = if long_link { EXTENDED_LINK } else { 0 };
    path.push(port.min(EXTENDED_PORT) as u8 | flags);
    if long_link {
        path.push(link.len() as u8);
    }
    if port >= EXTENDED_PORT {
        path.extend(port.to_le_bytes());
    }
    path.extend_from_slice(link);
    if path.len() % 2 != 0 {
        path.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_ports_encode_both_link_forms_and_preserve_alignment() {
        for (text, wire) in [
            ("14/255", &b"\x0e\xff"[..]),
            ("15/4", &b"\x0f\x0f\x00\x04"[..]),
            ("256/127.0.0.3", &b"\x1f\x09\x00\x01127.0.0.3\x00"[..]),
            ("65535/127.0.1.23", &b"\x1f\x0a\xff\xff127.0.1.23"[..]),
            (
                "1/2/300/192.0.2.100/15/7",
                &b"\x01\x02\x1f\x0b\x2c\x01192.0.2.100\x00\x0f\x0f\x00\x07"[..],
            ),
        ] {
            let route: Route = text.parse().unwrap();
            assert_eq!(route.to_string(), text);
            assert_eq!(route.port_segments(), wire);
        }
    }

    #[test]
    fn hex_link_bytes_preserve_padding_and_route_identity() {
        let route: Route = "300/hex:0080fe/1/9".parse().unwrap();
        assert_eq!(route.to_string(), "300/hex:0080FE/1/9");
        assert_eq!(
            route.port_segments(),
            b"\x1f\x03\x2c\x01\x00\x80\xfe\x00\x01\x09"
        );
        assert_eq!(route.to_string().parse::<Route>().unwrap(), route);
        assert_eq!(
            "2/hex:3139322e302e322e313030".parse::<Route>().unwrap(),
            "2/192.0.2.100".parse::<Route>().unwrap(),
        );
    }

    #[test]
    fn raw_links_and_padded_route_fit_their_wire_lengths() {
        let route = format!("1/hex:{}/1/hex:{}", "ab".repeat(255), "cd".repeat(250));
        let parsed: Route = route.parse().unwrap();
        assert_eq!(parsed.port_segments().len(), MAX_CIP_PATH_BYTES);
        assert_eq!(parsed.wrap(&[])[10], 255);
        assert!(format!("{route}/1/1").parse::<Route>().is_err());
        assert!(
            format!("1/hex:{}", "ab".repeat(256))
                .parse::<Route>()
                .is_err()
        );
    }
}

/// Distinguishes a rejected route from an attribute rejection at the destination.
#[derive(Debug)]
pub enum RoutedReadError {
    Exchange(EipError),
    Router(EipError),
    Target(EipError),
}

impl fmt::Display for RoutedReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exchange(error) => write!(f, "Routed read exchange: {error}"),
            Self::Router(error) => write!(f, "CIP router rejected Unconnected Send: {error}"),
            Self::Target(error) => write!(f, "CIP target rejected read: {error}"),
        }
    }
}

impl std::error::Error for RoutedReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Exchange(error) | Self::Router(error) | Self::Target(error) => error,
        })
    }
}

pub(super) fn decode_routed_reply<T>(
    raw: &[u8],
    decode: impl FnOnce(&[u8]) -> super::Result<T>,
) -> Result<T, RoutedReadError> {
    use RoutedReadError::{Exchange, Router, Target};
    if raw.first() == Some(&(UNCONNECTED_SEND | 0x80)) {
        return Err(match super::reply_body(raw, UNCONNECTED_SEND) {
            Ok(_) => Exchange(protocol(
                "Unconnected Send reply omitted the target service",
            )),
            Err(error @ EipError::Cip { .. }) => Router(error),
            Err(error) => Exchange(error),
        });
    }
    decode(raw).map_err(|error| match error {
        error @ EipError::Cip { .. } => Target(error),
        error => Exchange(error),
    })
}
pub(super) fn link_object(bytes: &[u8]) -> Result<String, String> {
    let (size, path) = bytes
        .split_first_chunk()
        .ok_or("Link Object is missing its word count")?;
    let words = usize::from(u16::from_le_bytes(*size));
    if path.len() != words * 2 {
        return Err(format!(
            "Link Object declares {words} words, with {} path bytes",
            path.len()
        ));
    }
    Ok(crate::model::hex(path))
}
