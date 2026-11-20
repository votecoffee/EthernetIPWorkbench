use super::MAX_LARGE_ASSEMBLY_BYTES;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    Utf8,
}
impl Kind {
    pub const ALL: [Self; 12] = [
        Self::Bool,
        Self::U8,
        Self::I8,
        Self::U16,
        Self::I16,
        Self::U32,
        Self::I32,
        Self::U64,
        Self::I64,
        Self::F32,
        Self::F64,
        Self::Utf8,
    ];
    fn width(self) -> usize {
        match self {
            Self::Bool | Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
            Self::Utf8 => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteSwap {
    #[default]
    None,
    BytesInWords,
    WordOrder,
}

impl ByteSwap {
    pub const ALL: [Self; 3] = [Self::None, Self::BytesInWords, Self::WordOrder];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No swap",
            Self::BytesInWords => "Swap bytes within words",
            Self::WordOrder => "Reverse word order",
        }
    }

    fn apply(self, bytes: &mut [u8]) {
        if self == Self::WordOrder {
            bytes.reverse();
        }
        if self != Self::None {
            for pair in bytes.chunks_exact_mut(2) {
                pair.swap(0, 1);
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub offset: usize,
    pub kind: Kind,
    #[serde(default)]
    pub big_endian: bool,
    pub bit: Option<u8>,
    #[serde(default, skip_serializing_if = "is_no_swap")]
    pub swap: ByteSwap,
    pub string_length: Option<usize>,
}

fn is_no_swap(swap: &ByteSwap) -> bool {
    *swap == ByteSwap::None
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            offset: 0,
            kind: Kind::U32,
            big_endian: false,
            bit: None,
            swap: ByteSwap::None,
            string_length: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureField {
    pub name: String,
    pub selection: Selection,
}

#[derive(Serialize)]
pub struct InputPreview {
    pub input_assembly: u16,
    pub input_size: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_min_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_bytes: Option<usize>,
    pub payload_hex: String,
    pub fields: Vec<InputFieldPreview>,
}

#[derive(Serialize)]
pub struct InputFieldPreview {
    pub name: String,
    pub selection: Selection,
    pub raw_hex: String,
    pub value: String,
    pub included_in_csv: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<T = Vec<u8>> {
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float32(f32),
    Float64(f64),
    Text(T),
}
impl<T: AsRef<[u8]>> std::fmt::Display for Value<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(v) => v.fmt(f),
            Self::Unsigned(v) => v.fmt(f),
            Self::Signed(v) => v.fmt(f),
            Self::Float32(v) => v.fmt(f),
            Self::Float64(v) => v.fmt(f),
            Self::Text(v) => String::from_utf8_lossy(v.as_ref()).fmt(f),
        }
    }
}
impl Value<&[u8]> {
    pub fn into_owned(self) -> Value {
        match self {
            Self::Bool(v) => Value::Bool(v),
            Self::Unsigned(v) => Value::Unsigned(v),
            Self::Signed(v) => Value::Signed(v),
            Self::Float32(v) => Value::Float32(v),
            Self::Float64(v) => Value::Float64(v),
            Self::Text(v) => Value::Text(v.to_vec()),
        }
    }
}
impl Selection {
    pub fn validate(self, size: usize) -> Result<(), String> {
        let width = self.validated_width()?;
        if size
            .checked_sub(self.offset)
            .is_none_or(|left| left < width)
        {
            return Err("Selected value extends past the assembly".into());
        }
        Ok(())
    }

    pub(crate) fn validated_width(self) -> Result<usize, String> {
        if self.kind == Kind::Utf8 && self.bit.is_some() {
            return Err("UTF-8 fields cannot select a single bit".into());
        }
        let width = self.width()?;
        if self.swap == ByteSwap::BytesInWords
            && (!width.is_multiple_of(2) || self.kind == Kind::Utf8)
            || self.swap == ByteSwap::WordOrder
                && (width < 4 || !width.is_multiple_of(2) || self.kind == Kind::Utf8)
        {
            return Err("This swap needs a numeric field with at least two bytes per word".into());
        }
        Ok(width)
    }

    pub fn width(self) -> Result<usize, String> {
        if let Some(bit) = self.bit {
            if bit >= 8 {
                return Err("Bit index must be 0–7".into());
            }
            return Ok(1);
        }
        if self.kind == Kind::Utf8 {
            return self
                .string_length
                .filter(|&size| (1..=MAX_LARGE_ASSEMBLY_BYTES).contains(&size))
                .ok_or_else(|| "UTF-8 needs a fixed byte length between 1 and 509".into());
        }
        Ok(self.kind.width())
    }

    pub fn decode(self, bytes: &[u8]) -> Option<Value<&[u8]>> {
        self.validate(bytes.len()).ok()?;
        if let Some(bit) = self.bit {
            let set = (bytes[self.offset] >> bit) & 1;
            return Some(if self.kind == Kind::Bool {
                Value::Bool(set != 0)
            } else {
                Value::Unsigned(set.into())
            });
        }
        let width = self.width().ok()?;
        let source = &bytes[self.offset..self.offset + width];
        if self.kind == Kind::Utf8 {
            return Some(Value::Text(
                source.split(|&byte| byte == 0).next().unwrap_or_default(),
            ));
        }
        let mut bits = [0; 8];
        bits[..width].copy_from_slice(source);
        self.swap.apply(&mut bits[..width]);
        if self.big_endian {
            bits[..width].reverse();
        }
        let value = u64::from_le_bytes(bits);
        Some(match self.kind {
            Kind::Bool => Value::Bool(value != 0),
            Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 => {
                let shift = 64 - width * 8;
                Value::Signed(((value << shift) as i64) >> shift)
            }
            Kind::F32 => Value::Float32(f32::from_bits(value as u32)),
            Kind::F64 => Value::Float64(f64::from_bits(value)),
            Kind::U8 | Kind::U16 | Kind::U32 | Kind::U64 => Value::Unsigned(value),
            Kind::Utf8 => unreachable!(),
        })
    }

    pub fn encode(self, value: &str, bytes: &mut [u8]) -> Result<(), String> {
        self.validate(bytes.len())?;
        let target = &mut bytes[self.offset..self.offset + self.width()?];
        if let Some(bit) = self.bit {
            let mask = 1 << bit;
            let set = parse_bool(value)?;
            target[0] = (target[0] & !mask) | if set { mask } else { 0 };
            return Ok(());
        }
        if self.kind == Kind::Utf8 {
            if value.len() > target.len() || value.contains('\0') {
                return Err("Text must fit the fixed byte length and cannot contain NUL".into());
            }
            target.fill(0);
            target[..value.len()].copy_from_slice(value.as_bytes());
            return Ok(());
        }
        let value = value.trim();
        let invalid = || format!("Enter a valid {:?} value", self.kind);
        macro_rules! parse {
            ($ty:ty) => {
                value.parse::<$ty>().map_err(|_| invalid())?.to_le_bytes()
            };
        }
        match self.kind {
            Kind::Bool => target[0] = u8::from(parse_bool(value)?),
            Kind::U8 => target.copy_from_slice(&parse!(u8)),
            Kind::I8 => target.copy_from_slice(&parse!(i8)),
            Kind::U16 => target.copy_from_slice(&parse!(u16)),
            Kind::I16 => target.copy_from_slice(&parse!(i16)),
            Kind::U32 => target.copy_from_slice(&parse!(u32)),
            Kind::I32 => target.copy_from_slice(&parse!(i32)),
            Kind::U64 => target.copy_from_slice(&parse!(u64)),
            Kind::I64 => target.copy_from_slice(&parse!(i64)),
            Kind::F32 => target.copy_from_slice(&parse!(f32)),
            Kind::F64 => target.copy_from_slice(&parse!(f64)),
            Kind::Utf8 => unreachable!(),
        }
        if self.big_endian {
            target.reverse();
        }
        self.swap.apply(target);
        Ok(())
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    let value = value.trim();
    if value == "1" || value.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if value == "0" || value.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err("Enter true, false, 1, or 0".into())
    }
}
