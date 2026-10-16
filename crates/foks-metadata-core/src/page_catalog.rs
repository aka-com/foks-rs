//! Qualification-only bounded row catalog. No statement authorizes a caller.
//! Integer SQL results remain canonical decimal TEXT until the host converts
//! them to the exact binary i64 page representation.
use crate::page::{Column, IntegerRange, PageKind, PageSpec};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PageStatement {
    ProbeTypes = 1,
    ListInitial,
    ListMacStart,
    ListMacContinue,
    ListTimeStart,
    ListTimeContinue,
}

const INTEGER: Column = Column {
    kind: PageKind::Integer,
    maximum_bytes: 8,
    nullable: false,
};
const fn blob(maximum_bytes: u32, nullable: bool) -> Column {
    Column {
        kind: PageKind::Blob,
        maximum_bytes,
        nullable,
    }
}
const fn text(maximum_bytes: u32) -> Column {
    Column {
        kind: PageKind::Text,
        maximum_bytes,
        nullable: false,
    }
}

// Probes precede payloads, distinguishing a missing optional row from a
// wrong-type/oversized value suppressed by CASE. NULL is never a decoding repair.
const LIST_COLUMNS: &[Column] = &[
    blob(16, false),
    blob(32, false),
    INTEGER,
    blob(17, false),
    text(7),
    INTEGER,
    blob(4096, true),
    INTEGER,
    text(7),
    INTEGER,
    blob(65536, true),
];

// Const storage gives every descriptor its required static lifetime.
const PROBE_PARAMETERS: &[Column] = &[
    INTEGER,
    INTEGER,
    blob(64, false),
    text(64),
    blob(64, true),
    INTEGER,
];
const PROBE_COLUMNS: &[Column] = &[INTEGER, INTEGER, blob(64, false), text(64), blob(64, true)];
const INITIAL_PARAMETERS: &[Column] = &[blob(33, false), blob(16, false), INTEGER, INTEGER];
const MAC_START_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    blob(32, false),
    INTEGER,
    INTEGER,
];
const MAC_CONTINUE_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    blob(32, false),
    blob(16, false),
    INTEGER,
    INTEGER,
];
const TIME_START_PARAMETERS: &[Column] =
    &[blob(33, false), blob(16, false), INTEGER, INTEGER, INTEGER];
const TIME_CONTINUE_PARAMETERS: &[Column] = &[
    blob(33, false),
    blob(16, false),
    INTEGER,
    blob(16, false),
    INTEGER,
    INTEGER,
];

macro_rules! list_projection {
    () => {
        "SELECT c.dirent_id,c.name_mac,CAST(c.creation_time AS TEXT),c.node_id,typeof(d.exact_dirent),CAST(COALESCE(length(CAST(d.exact_dirent AS BLOB)),-1) AS TEXT),CASE WHEN typeof(d.exact_dirent)='blob' AND length(d.exact_dirent)<=4096 THEN d.exact_dirent ELSE NULL END,CAST(n.node_id IS NOT NULL AS TEXT),typeof(n.exact_node),CAST(COALESCE(length(CAST(n.exact_node AS BLOB)),-1) AS TEXT),CASE WHEN typeof(n.exact_node)='blob' AND length(n.exact_node)<=65536 THEN n.exact_node ELSE NULL END "
    };
}
macro_rules! current_join {
    () => {
        "JOIN kv_dirents d ON d.uid=c.uid AND d.parent_id=c.parent_id AND d.dirent_id=c.dirent_id AND d.version=c.version "
    };
}
macro_rules! node_join {
    ($include:literal) => {
        concat!("LEFT JOIN kv_nodes n ON CAST(",$include," AS INTEGER)=1 AND substr(c.node_id,1,1)=X'03' AND n.uid=c.uid AND n.node_id=c.node_id ")
    };
}

const INITIAL: &str =
    concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?3"),
    "WHERE c.uid=?1 AND c.parent_id=?2 ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?4 AS INTEGER)");
const MAC_START: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?4"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND c.name_mac>?3 ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?5 AS INTEGER)");
const MAC_CONTINUE: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_name ",current_join!(),node_join!("?5"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND (c.name_mac,c.dirent_id)>(?3,?4) ORDER BY c.name_mac,c.dirent_id LIMIT CAST(?6 AS INTEGER)");
const TIME_START: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_time ",current_join!(),node_join!("?4"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND c.creation_time>=CAST(?3 AS INTEGER) ORDER BY c.creation_time,c.dirent_id LIMIT CAST(?5 AS INTEGER)");
const TIME_CONTINUE: &str = concat!(list_projection!(),
    "FROM kv_dirent_current c INDEXED BY kv_dirent_current_time ",current_join!(),node_join!("?5"),
    "WHERE c.uid=?1 AND c.parent_id=?2 AND (c.creation_time,c.dirent_id)>(CAST(?3 AS INTEGER),?4) ORDER BY c.creation_time,c.dirent_id LIMIT CAST(?6 AS INTEGER)");

impl PageStatement {
    pub const ALL: [Self; 6] = [
        Self::ProbeTypes,
        Self::ListInitial,
        Self::ListMacStart,
        Self::ListMacContinue,
        Self::ListTimeStart,
        Self::ListTimeContinue,
    ];
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|statement| *statement as u32 == id)
    }
    pub fn spec(self) -> PageSpec {
        let (sql,parameters,columns,integer_ranges,limit_parameter):(_, &'static [_], &'static [_], &'static [_], _) = match self {
            Self::ProbeTypes => (
                "WITH RECURSIVE seq(n) AS (SELECT 0 WHERE CAST(?1 AS INTEGER)>0 UNION ALL SELECT n+1 FROM seq WHERE n+1<CAST(?1 AS INTEGER)) SELECT CAST(n AS TEXT),CAST(CAST(?2 AS INTEGER) AS TEXT),?3,?4,?5 FROM seq LIMIT CAST(?6 AS INTEGER)",
                PROBE_PARAMETERS,
                PROBE_COLUMNS,
                &[IntegerRange{parameter:0,minimum:0,maximum:64}], Some(5)),
            Self::ListInitial => (INITIAL,INITIAL_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:1}],Some(3)),
            Self::ListMacStart => (MAC_START,MAC_START_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:3,minimum:0,maximum:1}],Some(4)),
            Self::ListMacContinue => (MAC_CONTINUE,MAC_CONTINUE_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:4,minimum:0,maximum:1}],Some(5)),
            Self::ListTimeStart => (TIME_START,TIME_START_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:i64::MAX},IntegerRange{parameter:3,minimum:0,maximum:1}],Some(4)),
            Self::ListTimeContinue => (TIME_CONTINUE,TIME_CONTINUE_PARAMETERS,LIST_COLUMNS,&[IntegerRange{parameter:2,minimum:0,maximum:i64::MAX},IntegerRange{parameter:4,minimum:0,maximum:1}],Some(5)),
        };
        PageSpec {
            id: self as u32,
            sql,
            parameters,
            columns,
            maximum_rows: 64,
            maximum_bytes: 1_048_576,
            integer_ranges,
            limit_parameter,
        }
    }
}
