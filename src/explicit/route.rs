//! CIP port paths and Unconnected Send read replies.

use super::{EipError, is_unicast, protocol};
use serde::{Deserialize, Serialize};
use std::{fmt, net::Ipv4Addr, str::FromStr};

pub(super) const UNCONNECTED_SEND: u8 = 0x52;
const MAX_HOPS: usize = 8;

/// A validated list of port/link pairs, for example `1/3` for backplane slot 3.
/// Each port is numbered 1–65535. A link is a byte or a unicast IPv4 address.
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
    Ipv4(Ipv4Addr),
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
                .trim();
            let link =
                if let Ok(node) = link.parse::<u8>() {
                    Link::Node(node)
                } else {
                    Link::Ipv4(link.parse::<Ipv4Addr>().ok().filter(|ip| is_unicast(*ip))
                    .ok_or("CIP links need a node number from 0 to 255 or a unicast IPv4 address")?)
                };
            hops.push(Hop { port, link });
        }
        Ok(Self(hops))
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
            match hop.link {
                Link::Node(node) => write!(f, "{}/{node}", hop.port)?,
                Link::Ipv4(ip) => write!(f, "{}/{ip}", hop.port)?,
            }
        }
        Ok(())
    }
}

impl Route {
    pub(crate) fn port_segments(&self) -> Vec<u8> {
        let mut path = Vec::new();
        for hop in &self.0 {
            match hop.link {
                Link::Node(node) => append_port_segment(&mut path, hop.port, &[node]),
                Link::Ipv4(ip) => {
                    append_port_segment(&mut path, hop.port, ip.to_string().as_bytes())
                }
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
