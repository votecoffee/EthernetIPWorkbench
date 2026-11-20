use super::number;
use crate::explicit::Identity;
use serde::{Deserialize, Serialize};

/// Device identity advertised by the EDS, for comparison with a CIP readback.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IdentityComparison {
    pub compared_fields: usize,
    pub differences: Vec<IdentityDifference>,
}

#[derive(Debug, Serialize)]
pub struct IdentityDifference {
    pub field: &'static str,
    pub eds: u16,
    pub adapter: u16,
}

impl DeviceIdentity {
    pub(super) fn parse<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        let word = |key| u16::try_from(number(get(key)?)?).ok();
        let byte = |key| u8::try_from(number(get(key)?)?).ok();
        let declared = Self {
            vendor: word("vendcode"),
            device_type: word("prodtype"),
            product_code: word("prodcode"),
            major: byte("majrev"),
            minor: byte("minrev"),
            product_name: get("prodname").map(str::to_owned),
        };
        (declared != Self::default()).then_some(declared)
    }

    fn numeric_fields(&self) -> [(&'static str, Option<u16>); 5] {
        [
            ("Vendor ID", self.vendor),
            ("Device type", self.device_type),
            ("Product code", self.product_code),
            ("Major revision", self.major.map(u16::from)),
            ("Minor revision", self.minor.map(u16::from)),
        ]
    }

    pub fn declared_numbers(&self) -> impl Iterator<Item = (&'static str, u16)> {
        self.numeric_fields()
            .into_iter()
            .filter_map(|(field, value)| Some((field, value?)))
    }

    pub fn compare(&self, identity: &Identity) -> IdentityComparison {
        let adapter_values = [
            identity.vendor,
            identity.device_type,
            identity.code,
            u16::from(identity.major),
            u16::from(identity.minor),
        ];
        let differences = self
            .numeric_fields()
            .into_iter()
            .zip(adapter_values)
            .filter_map(|((field, declared), adapter)| {
                let eds = declared?;
                (eds != adapter).then_some(IdentityDifference {
                    field,
                    eds,
                    adapter,
                })
            })
            .collect();
        IdentityComparison {
            compared_fields: self.declared_numbers().count(),
            differences,
        }
    }
}
