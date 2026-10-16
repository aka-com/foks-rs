//! Pure bounds for completed UTF-8 SQLite storage values. These measurements do
//! not admit a database, connection, request, or write. Callers must establish
//! encoding, schema and source provenance separately and normalize SQL affinity
//! before supplying postimages; raw SQL bind types are not completed values.
mod generated;
#[cfg(test)]
mod tests;
use generated::data;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Component {
    Foks,
    Authority,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableId {
    component: Component,
    ordinal: u16,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordId {
    component: Component,
    ordinal: u16,
}
#[derive(Clone, Copy, Debug)]
pub struct StorageProfile {
    _private: (),
}
pub const HOSTED_SMALL_V1: StorageProfile = StorageProfile { _private: () };
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageType {
    Integer,
    Real,
    Blob,
    Text,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dimension {
    Null,
    Integer,
    Real,
    Blob { bytes: u64 },
    TextUtf8 { bytes: u64 },
}
#[derive(Clone, Copy, Debug)]
pub enum BorrowedValue<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Blob(&'a [u8]),
    TextUtf8(&'a str),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType {
    Null,
    Integer,
    Real,
    Blob,
    Text,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Violation {
    Count {
        expected: u16,
        observed: usize,
    },
    Null {
        column: u16,
    },
    Type {
        column: u16,
        expected: StorageType,
        observed: ValueType,
    },
    FieldBytes {
        column: u16,
        observed: u64,
        limit: u64,
    },
    RecordBytes {
        record: RecordId,
        observed: u64,
        limit: u64,
    },
    ArithmeticOverflow {
        record: RecordId,
    },
}
impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UTF-8 storage bound: {self:?}")
    }
}
impl std::error::Error for Violation {}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowBounds {
    pub table_record_bytes: u64,
    pub maximum_index_bytes: u64,
    pub maximum_index: Option<RecordId>,
    pub physical_records_checked: u16,
}

fn component(value: Component) -> &'static data::Component {
    &data::COMPONENTS[match value {
        Component::Foks => 0,
        Component::Authority => 1,
    }]
}
fn record_id(c: Component, name: &str) -> RecordId {
    let ordinal = component(c)
        .records
        .iter()
        .position(|r| r.id == name)
        .expect("reviewed static record reference");
    RecordId {
        component: c,
        ordinal: u16::try_from(ordinal).expect("reviewed record bound"),
    }
}
fn storage_type(value: data::StorageType) -> StorageType {
    match value {
        data::StorageType::Integer => StorageType::Integer,
        data::StorageType::Real => StorageType::Real,
        data::StorageType::Blob => StorageType::Blob,
        data::StorageType::Text => StorageType::Text,
    }
}
impl Dimension {
    fn kind(self) -> ValueType {
        match self {
            Self::Null => ValueType::Null,
            Self::Integer => ValueType::Integer,
            Self::Real => ValueType::Real,
            Self::Blob { .. } => ValueType::Blob,
            Self::TextUtf8 { .. } => ValueType::Text,
        }
    }
    fn storage_type(self) -> Option<data::StorageType> {
        match self {
            Self::Null => None,
            Self::Integer => Some(data::StorageType::Integer),
            Self::Real => Some(data::StorageType::Real),
            Self::Blob { .. } => Some(data::StorageType::Blob),
            Self::TextUtf8 { .. } => Some(data::StorageType::Text),
        }
    }
    fn payload(self) -> u64 {
        match self {
            Self::Null => 0,
            Self::Integer | Self::Real => 8,
            Self::Blob { bytes } | Self::TextUtf8 { bytes } => bytes,
        }
    }
}
impl BorrowedValue<'_> {
    pub fn dimension(self) -> Dimension {
        match self {
            Self::Null => Dimension::Null,
            Self::Integer(_) => Dimension::Integer,
            Self::Real(_) => Dimension::Real,
            Self::Blob(v) => Dimension::Blob {
                bytes: v.len() as u64,
            },
            Self::TextUtf8(v) => Dimension::TextUtf8 {
                bytes: v.len() as u64,
            },
        }
    }
}
impl StorageProfile {
    pub fn name(self) -> &'static str {
        data::PROFILE
    }
    pub fn source_format(self) -> u32 {
        data::SOURCE_FORMAT
    }
    pub fn required_encoding(self) -> &'static str {
        data::REQUIRED_ENCODING
    }
    pub fn source_pins(self) -> &'static [(&'static str, &'static str)] {
        data::SOURCE_PINS
    }
    pub fn manifest(self) -> Inventory {
        Inventory(&data::MANIFEST)
    }
    pub fn ownership(self) -> Inventory {
        Inventory(&data::OWNERSHIP)
    }
    pub fn descriptor(self, value: Component) -> ComponentDescriptor {
        ComponentDescriptor(value)
    }
    pub fn table(self, value: Component, name: &str) -> Option<TableId> {
        component(value)
            .tables
            .iter()
            .position(|t| t.name == name)
            .map(|n| TableId {
                component: value,
                ordinal: n as u16,
            })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ComponentDescriptor(Component);
impl ComponentDescriptor {
    pub fn format_version(self) -> u32 {
        component(self.0).format_version
    }
    pub fn name(self) -> &'static str {
        component(self.0).component
    }
    pub fn scope(self) -> &'static str {
        component(self.0).scope
    }
    pub fn value_limit(self) -> u64 {
        component(self.0).value_limit
    }
    pub fn record_limit(self) -> u64 {
        component(self.0).record_limit
    }
    pub fn record_formula(self) -> &'static str {
        component(self.0).record_formula
    }
    pub fn tables(self) -> impl ExactSizeIterator<Item = TableId> + Clone {
        (0..component(self.0).tables.len()).map(move |n| TableId {
            component: self.0,
            ordinal: n as u16,
        })
    }
    pub fn records(self) -> impl ExactSizeIterator<Item = RecordId> + Clone {
        (0..component(self.0).records.len()).map(move |n| RecordId {
            component: self.0,
            ordinal: n as u16,
        })
    }
}
impl TableId {
    fn data(self) -> &'static data::Table {
        &component(self.component).tables[usize::from(self.ordinal)]
    }
    pub fn component(self) -> Component {
        self.component
    }
    pub fn name(self) -> &'static str {
        self.data().name
    }
    pub fn sql_sha256(self) -> &'static str {
        self.data().sql_sha256
    }
    pub fn strict(self) -> bool {
        self.data().strict
    }
    pub fn without_rowid(self) -> bool {
        self.data().without_rowid
    }
    pub fn columns(self) -> impl ExactSizeIterator<Item = ColumnDescriptor> + Clone {
        self.data().columns.iter().map(ColumnDescriptor)
    }
    pub fn record(self) -> RecordId {
        record_id(self.component, self.data().record_id)
    }
    pub fn locator(self) -> Locator {
        Locator(&self.data().locator)
    }
    pub fn indexes(self) -> impl ExactSizeIterator<Item = IndexDescriptor> + Clone {
        self.data()
            .indexes
            .iter()
            .map(move |index| IndexDescriptor {
                component: self.component,
                index,
            })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ColumnDescriptor(&'static data::Column);
impl ColumnDescriptor {
    pub fn ordinal(self) -> u16 {
        self.0.ordinal
    }
    pub fn name(self) -> &'static str {
        self.0.name
    }
    pub fn declared_type(self) -> &'static str {
        self.0.declared_type
    }
    pub fn storage_type(self) -> StorageType {
        storage_type(self.0.storage_type)
    }
    pub fn nullable(self) -> bool {
        self.0.nullable
    }
    pub fn explicit_not_null(self) -> bool {
        self.0.explicit_not_null
    }
    pub fn primary_key_ordinal(self) -> u16 {
        self.0.primary_key_ordinal
    }
    pub fn integer_rowid_alias(self) -> bool {
        self.0.integer_rowid_alias
    }
    pub fn maximum_bytes(self) -> Option<u64> {
        self.0.maximum_bytes
    }
    pub fn policy_source(self) -> &'static str {
        self.0.policy_source
    }
}
#[derive(Clone, Copy, Debug)]
pub struct IndexDescriptor {
    component: Component,
    index: &'static data::Index,
}
impl IndexDescriptor {
    pub fn name(self) -> &'static str {
        self.index.name
    }
    pub fn origin(self) -> &'static str {
        self.index.origin
    }
    pub fn unique(self) -> bool {
        self.index.unique
    }
    pub fn partial(self) -> bool {
        self.index.partial
    }
    pub fn partial_policy(self) -> &'static str {
        self.index.partial_policy
    }
    pub fn sql_sha256(self) -> Option<&'static str> {
        self.index.sql_sha256
    }
    pub fn record(self) -> RecordId {
        record_id(self.component, self.index.record_id)
    }
    pub fn table_btree_alias(self) -> bool {
        self.index.table_btree_alias
    }
}
impl RecordId {
    fn data(self) -> &'static data::Record {
        &component(self.component).records[usize::from(self.ordinal)]
    }
    pub fn component(self) -> Component {
        self.component
    }
    pub fn name(self) -> &'static str {
        self.data().id
    }
    pub fn table(self) -> &'static str {
        self.data().table
    }
    pub fn kind(self) -> &'static str {
        self.data().kind
    }
    pub fn operands(self) -> impl ExactSizeIterator<Item = Operand> + Clone {
        self.data().operands.iter().map(Operand)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Operand(&'static data::Operand);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperandSource {
    Column { ordinal: u16 },
    Rowid,
    NodeFileSlice { ordinal: u16, maximum_bytes: u64 },
    ScopedHostOrEmpty { ordinal: u16, maximum_bytes: u64 },
    IntegerOne { maximum_bytes: u64 },
}
impl Operand {
    pub fn source(self) -> OperandSource {
        match self.0.source {
            data::Source::Column { ordinal } => OperandSource::Column { ordinal },
            data::Source::Rowid => OperandSource::Rowid,
            data::Source::NodeFileSlice {
                ordinal,
                maximum_bytes,
            } => OperandSource::NodeFileSlice {
                ordinal,
                maximum_bytes,
            },
            data::Source::ScopedHostOrEmpty {
                ordinal,
                maximum_bytes,
            } => OperandSource::ScopedHostOrEmpty {
                ordinal,
                maximum_bytes,
            },
            data::Source::IntegerOne { maximum_bytes } => {
                OperandSource::IntegerOne { maximum_bytes }
            }
        }
    }
    pub fn descending(self) -> bool {
        self.0.descending
    }
    pub fn collation(self) -> &'static str {
        self.0.collation
    }
    pub fn key(self) -> bool {
        self.0.key
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Locator(&'static data::Locator);
impl Locator {
    pub fn rowid_spelling(self) -> Option<&'static str> {
        match self.0 {
            data::Locator::Rowid { spelling } => Some(spelling),
            _ => None,
        }
    }
    pub fn primary_key(self) -> Option<impl ExactSizeIterator<Item = Operand> + Clone> {
        match self.0 {
            data::Locator::PrimaryKey { operands } => Some(operands.iter().map(Operand)),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Inventory(&'static data::Inventory);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryKind {
    Null,
    Bool,
    Unsigned,
    Text,
    Array,
    Object,
}
impl Inventory {
    pub fn kind(self) -> InventoryKind {
        match self.0 {
            data::Inventory::Null => InventoryKind::Null,
            data::Inventory::Bool(_) => InventoryKind::Bool,
            data::Inventory::Unsigned(_) => InventoryKind::Unsigned,
            data::Inventory::Text(_) => InventoryKind::Text,
            data::Inventory::Array(_) => InventoryKind::Array,
            data::Inventory::Object(_) => InventoryKind::Object,
        }
    }
    pub fn boolean(self) -> Option<bool> {
        match self.0 {
            data::Inventory::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub fn unsigned(self) -> Option<u64> {
        match self.0 {
            data::Inventory::Unsigned(v) => Some(*v),
            _ => None,
        }
    }
    pub fn text(self) -> Option<&'static str> {
        match self.0 {
            data::Inventory::Text(v) => Some(v),
            _ => None,
        }
    }
    pub fn elements(self) -> Option<impl ExactSizeIterator<Item = Inventory> + Clone> {
        match self.0 {
            data::Inventory::Array(v) => Some(v.iter().map(Inventory)),
            _ => None,
        }
    }
    pub fn members(
        self,
    ) -> Option<impl ExactSizeIterator<Item = (&'static str, Inventory)> + Clone> {
        match self.0 {
            data::Inventory::Object(v) => Some(v.iter().map(|(k, v)| (*k, Inventory(v)))),
            _ => None,
        }
    }
}

fn validate_fields(columns: &[data::Column], values: &[Dimension]) -> Result<(), Violation> {
    if values.len() != columns.len() {
        return Err(Violation::Count {
            expected: columns.len() as u16,
            observed: values.len(),
        });
    }
    for (column, value) in columns.iter().zip(values) {
        let Some(actual) = value.storage_type() else {
            if !column.nullable {
                return Err(Violation::Null {
                    column: column.ordinal,
                });
            }
            continue;
        };
        if std::mem::discriminant(&actual) != std::mem::discriminant(&column.storage_type) {
            return Err(Violation::Type {
                column: column.ordinal,
                expected: storage_type(column.storage_type),
                observed: value.kind(),
            });
        }
        if let Some(limit) = column.maximum_bytes {
            if value.payload() > limit {
                return Err(Violation::FieldBytes {
                    column: column.ordinal,
                    observed: value.payload(),
                    limit,
                });
            }
        }
    }
    Ok(())
}
fn record_bound(operands: &[data::Operand], values: &[Dimension]) -> Option<u64> {
    let mut bound = 9u64.checked_add(9u64.checked_mul(u64::try_from(operands.len()).ok()?)?)?;
    for operand in operands {
        let payload = match operand.source {
            data::Source::Column { ordinal } => values.get(usize::from(ordinal))?.payload(),
            data::Source::Rowid | data::Source::IntegerOne { .. } => 8,
            data::Source::NodeFileSlice {
                ordinal,
                maximum_bytes,
            } => values
                .get(usize::from(ordinal))?
                .payload()
                .saturating_sub(1)
                .min(maximum_bytes),
            data::Source::ScopedHostOrEmpty {
                ordinal,
                maximum_bytes,
            } => values
                .get(usize::from(ordinal))?
                .payload()
                .min(maximum_bytes),
        };
        bound = bound.checked_add(payload)?;
    }
    Some(bound)
}
fn checked_record(record: RecordId, values: &[Dimension]) -> Result<u64, Violation> {
    let observed = record_bound(record.data().operands, values)
        .ok_or(Violation::ArithmeticOverflow { record })?;
    let limit = component(record.component).record_limit;
    if observed > limit {
        return Err(Violation::RecordBytes {
            record,
            observed,
            limit,
        });
    }
    Ok(observed)
}
/// Values must describe the completed, affinity-normalized row in UTF-8 storage.
/// Success measures sizes only and does not bind the caller's dimensions to data.
pub fn check_utf8_row_dimensions(
    table: TableId,
    values: &[Dimension],
) -> Result<RowBounds, Violation> {
    validate_fields(table.data().columns, values)?;
    let table_record_bytes = checked_record(table.record(), values)?;
    let mut result = RowBounds {
        table_record_bytes,
        maximum_index_bytes: 0,
        maximum_index: None,
        physical_records_checked: 1,
    };
    for index in table.indexes() {
        if index.table_btree_alias() {
            continue;
        }
        let record = index.record();
        let bound = checked_record(record, values)?;
        result.physical_records_checked = result
            .physical_records_checked
            .checked_add(1)
            .ok_or(Violation::ArithmeticOverflow { record })?;
        if bound > result.maximum_index_bytes {
            result.maximum_index_bytes = bound;
            result.maximum_index = Some(record);
        }
    }
    Ok(result)
}
/// Borrowed completed values, not raw SQL bindings. No payload is copied.
pub fn check_utf8_row(
    table: TableId,
    values: &[BorrowedValue<'_>],
) -> Result<RowBounds, Violation> {
    let expected = table.data().columns.len();
    if values.len() != expected {
        return Err(Violation::Count {
            expected: expected as u16,
            observed: values.len(),
        });
    }
    let mut dimensions = [Dimension::Null; 32];
    for (target, value) in dimensions.iter_mut().zip(values) {
        *target = value.dimension();
    }
    check_utf8_row_dimensions(table, &dimensions[..expected])
}
