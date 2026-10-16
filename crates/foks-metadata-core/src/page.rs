//! Private bounded row codec. This is not a FOKS public wire format.
use crate::{Error, Result, Value};
use serde::Serialize;

pub const ABI_VERSION: u16 = 1;
pub const HEADER_BYTES: usize = 24;
pub const MAX_PAGE_BYTES: usize = 1024 * 1024;
pub const MAX_PARAMETER_BYTES: usize = 64 * 1024;
pub const MAX_ROWS: u16 = 64;
pub const MAX_COLUMNS: u16 = 16;

/// Immutable deployment artifact; the module exports the same complete contract.
pub fn catalog_artifact() -> Result<Vec<u8>> {
    use sha2::{Digest as _, Sha256};
    let specifications: Vec<_> = crate::page_catalog::PageStatement::ALL
        .iter()
        .map(|s| s.spec())
        .collect();
    for spec in &specifications {
        spec.validate()?;
    }
    let descriptor = serde_json::json!({"abi_version":ABI_VERSION,"statements":specifications});
    let encoded = serde_json::to_vec(&descriptor).map_err(|_| Error::Bounds)?;
    let fingerprint: String = Sha256::digest(&encoded)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bytes =
        serde_json::to_vec(&serde_json::json!({"descriptor":descriptor,"sha256":fingerprint}))
            .map_err(|_| Error::Bounds)?;
    if bytes.len() > MAX_PARAMETER_BYTES {
        return Err(Error::Bounds);
    }
    Ok(bytes)
}

pub(crate) fn charge(bytes: usize) -> Result<()> {
    foks_snowpack::reserve_allocation(bytes).map_err(|_| Error::Bounds)
}

pub fn decode_parameters(
    bytes: Vec<u8>,
) -> Result<(crate::page_catalog::PageStatement, Vec<Value>)> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_PARAMETER_BYTES {
        return Err(Error::Bounds);
    }
    let id = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| Error::Bounds)?);
    let statement = crate::page_catalog::PageStatement::from_id(id).ok_or(Error::Bounds)?;
    let spec = statement.spec();
    let page = PageBuffer::decode(bytes, &spec, EnvelopeKind::Parameters)?;
    charge(
        page.as_bytes()
            .len()
            .checked_add(spec.parameters.len() * std::mem::size_of::<Value>())
            .ok_or(Error::Bounds)?,
    )?;
    let values = page
        .rows()
        .next()
        .ok_or(Error::Bounds)?
        .cells()
        .map(|cell| match cell {
            Cell::Null => Value::Null,
            Cell::Integer(n) => Value::Integer(n),
            Cell::Blob(b) => Value::Blob(b.to_vec()),
            Cell::Text(s) => Value::Text(s.to_owned()),
        })
        .collect::<Vec<_>>();
    spec.validate_parameters(&values)?;
    Ok((statement, values))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Integer,
    Blob,
    Text,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Column {
    pub kind: PageKind,
    /// Payload bytes, excluding tag and length prefix; Integer is exactly eight.
    pub maximum_bytes: u32,
    pub nullable: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct IntegerRange {
    pub parameter: u16,
    #[serde(with = "crate::catalog::decimal")]
    pub minimum: i64,
    #[serde(with = "crate::catalog::decimal")]
    pub maximum: i64,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PageSpec {
    pub id: u32,
    pub sql: &'static str,
    pub parameters: &'static [Column],
    pub columns: &'static [Column],
    pub maximum_rows: u16,
    pub maximum_bytes: u32,
    pub integer_ranges: &'static [IntegerRange],
    pub limit_parameter: Option<u16>,
}

impl Column {
    fn encoded_maximum(self) -> Result<usize> {
        match self.kind {
            PageKind::Integer if self.maximum_bytes == 8 => Ok(9),
            PageKind::Integer => Err(Error::Bounds),
            PageKind::Blob | PageKind::Text => usize::try_from(self.maximum_bytes)
                .map_err(|_| Error::Bounds)?
                .checked_add(5)
                .ok_or(Error::Bounds),
        }
    }
    fn accepts(self, value: Cell<'_>) -> Result<()> {
        match value {
            Cell::Null if self.nullable => Ok(()),
            Cell::Integer(_) if self.kind == PageKind::Integer => Ok(()),
            Cell::Blob(bytes)
                if self.kind == PageKind::Blob && bytes.len() <= self.maximum_bytes as usize =>
            {
                Ok(())
            }
            Cell::Text(text)
                if self.kind == PageKind::Text && text.len() <= self.maximum_bytes as usize =>
            {
                Ok(())
            }
            _ => Err(Error::Bounds),
        }
    }
}

impl PageSpec {
    pub fn validate(&self) -> Result<()> {
        if self.id == 0
            || self.sql.is_empty()
            || self.parameters.len() > usize::from(MAX_COLUMNS)
            || self.columns.is_empty()
            || self.columns.len() > usize::from(MAX_COLUMNS)
            || self.maximum_rows == 0
            || self.maximum_rows > MAX_ROWS
            || self.maximum_bytes as usize > MAX_PAGE_BYTES
            || self.maximum_bytes as usize <= HEADER_BYTES
        {
            return Err(Error::Bounds);
        }
        for column in self.parameters.iter().chain(self.columns) {
            column.encoded_maximum()?;
        }
        let parameters = self.parameters.iter().try_fold(HEADER_BYTES, |n, c| {
            n.checked_add(c.encoded_maximum()?).ok_or(Error::Bounds)
        })?;
        if parameters > MAX_PARAMETER_BYTES || self.effective_rows_unchecked()? == 0 {
            return Err(Error::Bounds);
        }
        for range in self.integer_ranges {
            let column = self
                .parameters
                .get(usize::from(range.parameter))
                .ok_or(Error::Bounds)?;
            if column.kind != PageKind::Integer || column.nullable || range.minimum > range.maximum
            {
                return Err(Error::Bounds);
            }
        }
        if let Some(index) = self.limit_parameter {
            let column = self
                .parameters
                .get(usize::from(index))
                .ok_or(Error::Bounds)?;
            if column.kind != PageKind::Integer || column.nullable {
                return Err(Error::Bounds);
            }
        }
        Ok(())
    }
    pub fn maximum_row_bytes(&self) -> Result<usize> {
        self.columns.iter().try_fold(0usize, |n, c| {
            n.checked_add(c.encoded_maximum()?).ok_or(Error::Bounds)
        })
    }
    fn effective_rows_unchecked(&self) -> Result<u16> {
        let body = (self.maximum_bytes as usize)
            .checked_sub(HEADER_BYTES)
            .ok_or(Error::Bounds)?;
        let row = self.maximum_row_bytes()?;
        if row == 0 {
            return Err(Error::Bounds);
        }
        u16::try_from((body / row).min(usize::from(self.maximum_rows))).map_err(|_| Error::Bounds)
    }
    pub fn effective_rows(&self) -> Result<u16> {
        self.validate()?;
        self.effective_rows_unchecked()
    }
    pub fn validate_parameters(&self, values: &[Value]) -> Result<()> {
        self.validate()?;
        if values.len() != self.parameters.len() {
            return Err(Error::Bounds);
        }
        for (column, value) in self.parameters.iter().zip(values) {
            column.accepts(Cell::from(value))?;
        }
        for range in self.integer_ranges {
            let Value::Integer(value) = values[usize::from(range.parameter)] else {
                return Err(Error::Bounds);
            };
            if value < range.minimum || value > range.maximum {
                return Err(Error::Bounds);
            }
        }
        self.requested_rows(values)?;
        Ok(())
    }
    pub fn requested_rows(&self, values: &[Value]) -> Result<u16> {
        let maximum = self.effective_rows()?;
        let Some(index) = self.limit_parameter else {
            return Ok(maximum);
        };
        let Some(Value::Integer(limit)) = values.get(usize::from(index)) else {
            return Err(Error::Bounds);
        };
        let limit = u16::try_from(*limit).map_err(|_| Error::Bounds)?;
        if limit == 0 || limit > maximum {
            return Err(Error::Bounds);
        }
        Ok(limit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cell<'a> {
    Null,
    Integer(i64),
    Blob(&'a [u8]),
    Text(&'a str),
}
impl<'a> From<&'a Value> for Cell<'a> {
    fn from(value: &'a Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Integer(value) => Self::Integer(*value),
            Value::Blob(value) => Self::Blob(value),
            Value::Text(value) => Self::Text(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum EnvelopeKind {
    Parameters = 1,
    Rows = 2,
}

fn header(spec: &PageSpec, kind: EnvelopeKind, columns: usize) -> Result<Vec<u8>> {
    reserved_header(spec, kind, columns, HEADER_BYTES)
}
fn reserved_header(
    spec: &PageSpec,
    kind: EnvelopeKind,
    columns: usize,
    capacity: usize,
) -> Result<Vec<u8>> {
    spec.validate()?;
    if !(HEADER_BYTES..=MAX_PAGE_BYTES).contains(&capacity) {
        return Err(Error::Bounds);
    }
    charge(capacity)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::Bounds)?;
    bytes.extend_from_slice(b"FKPG");
    bytes.extend_from_slice(&ABI_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(kind as u16).to_le_bytes());
    bytes.extend_from_slice(&spec.id.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&(columns as u16).to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    Ok(bytes)
}

fn append_row(
    bytes: &mut Vec<u8>,
    columns: &[Column],
    cells: &[Cell<'_>],
    maximum: usize,
) -> Result<()> {
    if columns.len() != cells.len() {
        return Err(Error::Bounds);
    }
    let mut needed = 0usize;
    for (column, cell) in columns.iter().zip(cells) {
        column.accepts(*cell)?;
        let length = match cell {
            Cell::Null => 1,
            Cell::Integer(_) => 9,
            Cell::Blob(bytes) => bytes.len().checked_add(5).ok_or(Error::Bounds)?,
            Cell::Text(text) => text.len().checked_add(5).ok_or(Error::Bounds)?,
        };
        needed = needed.checked_add(length).ok_or(Error::Bounds)?;
    }
    if bytes.len().checked_add(needed).ok_or(Error::Bounds)? > maximum {
        return Err(Error::Bounds);
    }
    // Charge the new allocation including existing bytes when growth reallocates.
    // This is conservative cumulative admission, not a measured peak.
    let total = bytes.len().checked_add(needed).ok_or(Error::Bounds)?;
    if total > bytes.capacity() {
        charge(total)?;
    }
    bytes.try_reserve_exact(needed).map_err(|_| Error::Bounds)?;
    for cell in cells {
        match cell {
            Cell::Null => bytes.push(0),
            Cell::Integer(n) => {
                bytes.push(1);
                bytes.extend_from_slice(&n.to_le_bytes());
            }
            Cell::Blob(value) => {
                bytes.push(2);
                bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
                bytes.extend_from_slice(value);
            }
            Cell::Text(value) => {
                bytes.push(3);
                bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
                bytes.extend_from_slice(value.as_bytes());
            }
        }
    }
    Ok(())
}

pub fn encode_parameters(spec: &PageSpec, values: &[Value]) -> Result<Vec<u8>> {
    spec.validate_parameters(values)?;
    let mut bytes = header(spec, EnvelopeKind::Parameters, spec.parameters.len())?;
    charge(values.len() * std::mem::size_of::<Cell<'_>>())?;
    let cells: Vec<_> = values.iter().map(Cell::from).collect();
    append_row(&mut bytes, spec.parameters, &cells, MAX_PARAMETER_BYTES)?;
    finish_header(&mut bytes, 1)?;
    Ok(bytes)
}

fn finish_header(bytes: &mut [u8], rows: u16) -> Result<()> {
    let body = u32::try_from(bytes.len().checked_sub(HEADER_BYTES).ok_or(Error::Bounds)?)
        .map_err(|_| Error::Bounds)?;
    bytes[12..14].copy_from_slice(&rows.to_le_bytes());
    bytes[16..20].copy_from_slice(&body.to_le_bytes());
    Ok(())
}

pub struct PageEncoder {
    spec: PageSpec,
    bytes: Vec<u8>,
    rows: u16,
    limit: u16,
}
impl PageEncoder {
    pub fn new(spec: PageSpec, limit: u16) -> Result<Self> {
        if limit == 0 || limit > spec.effective_rows()? {
            return Err(Error::Bounds);
        }
        let capacity = spec
            .maximum_row_bytes()?
            .checked_mul(usize::from(limit))
            .and_then(|body| body.checked_add(HEADER_BYTES))
            .ok_or(Error::Bounds)?;
        if capacity > spec.maximum_bytes as usize {
            return Err(Error::Bounds);
        }
        Ok(Self {
            bytes: reserved_header(&spec, EnvelopeKind::Rows, spec.columns.len(), capacity)?,
            spec,
            rows: 0,
            limit,
        })
    }
    pub fn push(&mut self, cells: &[Cell<'_>]) -> Result<()> {
        if self.rows >= self.limit {
            return Err(Error::Bounds);
        }
        append_row(
            &mut self.bytes,
            self.spec.columns,
            cells,
            self.spec.maximum_bytes as usize,
        )?;
        self.rows += 1;
        Ok(())
    }
    pub fn finish(mut self) -> Result<PageBuffer> {
        finish_header(&mut self.bytes, self.rows)?;
        PageBuffer::decode(self.bytes, &self.spec, EnvelopeKind::Rows)
    }
}

/// One owned bounded buffer; views never borrow a SQL connection or host buffer.
#[derive(Debug)]
pub struct PageBuffer {
    bytes: Vec<u8>,
    rows: u16,
    columns: u16,
}
impl PageBuffer {
    pub fn decode(bytes: Vec<u8>, spec: &PageSpec, kind: EnvelopeKind) -> Result<Self> {
        spec.validate()?;
        let (maximum, columns, maximum_rows) = match kind {
            EnvelopeKind::Parameters => (MAX_PARAMETER_BYTES, spec.parameters, 1),
            EnvelopeKind::Rows => (
                spec.maximum_bytes as usize,
                spec.columns,
                spec.effective_rows()?,
            ),
        };
        if bytes.len() < HEADER_BYTES || bytes.len() > maximum || &bytes[..4] != b"FKPG" {
            return Err(Error::Bounds);
        }
        let u16_at = |n| u16::from_le_bytes(bytes[n..n + 2].try_into().expect("checked header"));
        let u32_at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().expect("checked header"));
        let rows = u16_at(12);
        if u16_at(4) != ABI_VERSION
            || u16_at(6) != kind as u16
            || u32_at(8) != spec.id
            || rows > maximum_rows
            || (kind == EnvelopeKind::Parameters && rows != 1)
            || usize::from(u16_at(14)) != columns.len()
            || u32_at(16) as usize != bytes.len() - HEADER_BYTES
            || u32_at(20) != 0
        {
            return Err(Error::Bounds);
        }
        let mut input = &bytes[HEADER_BYTES..];
        for _ in 0..rows {
            for column in columns {
                column.accepts(next_cell(&mut input)?)?;
            }
        }
        if !input.is_empty() {
            return Err(Error::Bounds);
        }
        Ok(Self {
            columns: columns.len() as u16,
            bytes,
            rows,
        })
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn len(&self) -> usize {
        usize::from(self.rows)
    }
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }
    pub fn rows(&self) -> Rows<'_> {
        Rows {
            input: &self.bytes[HEADER_BYTES..],
            remaining: self.rows,
            columns: self.columns,
        }
    }
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    if length > input.len() {
        return Err(Error::Bounds);
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    Ok(value)
}
fn next_cell<'a>(input: &mut &'a [u8]) -> Result<Cell<'a>> {
    match take(input, 1)?[0] {
        0 => Ok(Cell::Null),
        1 => Ok(Cell::Integer(i64::from_le_bytes(
            take(input, 8)?.try_into().map_err(|_| Error::Bounds)?,
        ))),
        tag @ (2 | 3) => {
            let length =
                u32::from_le_bytes(take(input, 4)?.try_into().map_err(|_| Error::Bounds)?) as usize;
            let bytes = take(input, length)?;
            if tag == 2 {
                Ok(Cell::Blob(bytes))
            } else {
                Ok(Cell::Text(
                    std::str::from_utf8(bytes).map_err(|_| Error::Bounds)?,
                ))
            }
        }
        _ => Err(Error::Bounds),
    }
}
pub struct Rows<'a> {
    input: &'a [u8],
    remaining: u16,
    columns: u16,
}
pub struct Row<'a> {
    input: &'a [u8],
    columns: u16,
}
pub struct Cells<'a> {
    input: &'a [u8],
    remaining: u16,
}
impl<'a> Iterator for Rows<'a> {
    type Item = Row<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let input = self.input;
        for _ in 0..self.columns {
            next_cell(&mut self.input).expect("validated immutable page");
        }
        self.remaining -= 1;
        Some(Row {
            input: &input[..input.len() - self.input.len()],
            columns: self.columns,
        })
    }
}
impl<'a> Row<'a> {
    pub fn cells(&self) -> Cells<'a> {
        Cells {
            input: self.input,
            remaining: self.columns,
        }
    }
}
impl<'a> Iterator for Cells<'a> {
    type Item = Cell<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(next_cell(&mut self.input).expect("validated immutable page"))
    }
}
