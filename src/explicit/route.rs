//! CIP port paths and Unconnected Send read replies.

use super::{EipError, is_unicast, protocol};
use serde::{Deserialize, Serialize};
use std::{fmt, net::Ipv4Addr, str::FromStr};

pub(super) const UNCONNECTED_SEND: u8 = 0x52;
const MAX_HOPS: usize = 8;

/// A validated list of port/link pairs, for example `1/3` for backplane slot 3.
/// Each port is numbered 1–14. A link is a byte or a unicast IPv4 address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Route(Vec<Hop>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hop {
    port: u8,
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
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=14).contains(n))
                .ok_or("Enter a CIP port number from 1 to 14")?;
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
                Link::Node(node) => path.extend([hop.port, node]),
                Link::Ipv4(ip) => {
                    let address = ip.to_string();
                    path.extend([hop.port | 0x10, address.len() as u8]);
                    path.extend(address.bytes());
                    if address.len() % 2 != 0 {
                        path.push(0);
                    }
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

impl RoutedReadError {
    pub(super) fn from_reply(raw: &[u8], service: u8) -> Result<Vec<u8>, Self> {
        let router = raw.first() == Some(&(UNCONNECTED_SEND | 0x80));
        let expected = if router { UNCONNECTED_SEND } else { service };
        match super::reply_body(raw, expected) {
            Ok(_) if router => Err(Self::Exchange(protocol(
                "Unconnected Send reply omitted the target service",
            ))),
            Ok(body) => Ok(body.to_vec()),
            Err(error @ EipError::Cip { .. }) if router => Err(Self::Router(error)),
            Err(error @ EipError::Cip { .. }) => Err(Self::Target(error)),
            Err(error) => Err(Self::Exchange(error)),
        }
    }
}
