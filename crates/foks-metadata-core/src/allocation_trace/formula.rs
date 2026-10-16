//! Checked, independent valid-schema charge bounds; not a runtime allowance.
use super::{layout, Layout};
use crate::{page_catalog::PageStatement as P, Error, Result};
use foks_proto::{
    KvDirectoryStatus, KvDirent, KvNodeId, KvSmallFileBox, Role, RoleAndGeneration, SecretBox,
};
use foks_snowpack::Value;
use serde::Serialize;
#[derive(Serialize)]
pub struct Term {
    pub site: &'static str,
    pub scope: &'static str,
    pub bytes: u64,
}
#[derive(Serialize)]
pub struct Bounds {
    pub complete: bool,
    pub page_transport: u64,
    pub row_decode: u64,
    pub row_append: u64,
    pub page_roles: u64,
    pub full_page_scratch: u64,
    pub retained_growth: u64,
    pub finish: u64,
    pub parent: u64,
    pub frame: u64,
    pub terms: [Term; 12],
    pub uncovered: [&'static str; 6],
}
fn add(values: &[usize]) -> Result<usize> {
    values
        .iter()
        .try_fold(0usize, |a, b| a.checked_add(*b).ok_or(Error::Bounds))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Error::Bounds)
}
// Independent accounting of Snowpack decoder and owned encoder rules.
pub(super) fn shape(value: &Value, v: usize) -> Result<(usize, usize)> {
    match value {
        Value::Binary(b) | Value::Text(b) => {
            Ok((add(&[mul(4, b.len())?, 64])?, add(&[9, b.len()])?))
        }
        Value::Array(a) => a.iter().try_fold(
            (add(&[mul(mul(4, a.len())?, v)?, 4096])?, 9),
            |(d, e), x| {
                let (b, c) = shape(x, v)?;
                Ok((add(&[d, b])?, add(&[e, c])?))
            },
        ),
        Value::Variant(Some((tag, x))) => {
            let (d, e) = shape(x, v)?;
            Ok((
                add(&[mul(4, v)?, 128, mul(4, tag.len())?, 64, d])?,
                add(&[9, tag.len(), e])?,
            ))
        }
        _ => Ok((0, 9)),
    }
}
fn query(p: P, rows: usize, parameters: usize, payload: usize, l: &Layout) -> Result<usize> {
    let spec = p.spec();
    let row = spec.maximum_row_bytes()?;
    let output = add(&[24, mul(rows, row)?])?;
    if let Some(t) = l.sql_value_bytes {
        add(&[
            payload,
            mul(parameters, t)?,
            mul(
                spec.columns.len(),
                add(&[512, mul(add(&[rows, 1])?, l.cell_bytes)?])?,
            )?,
            row,
            output,
        ])
    } else {
        // Header, one parameter-buffer growth, borrowed parameter cells, reply.
        let encoded = add(&[24, payload, mul(parameters, 5)?])?;
        add(&[24, encoded, mul(parameters, l.cell_bytes)?, output])
    }
}
fn schemas() -> Result<(KvDirent, KvSmallFileBox)> {
    // Member is the longest valid role shape; whole-record caps overestimate
    // ciphertext lengths. Scalar width does not change these charge formulas.
    let entry = KvDirent::new(
        [0; 16],
        [0; 16],
        KvNodeId([3; 17]),
        u64::MAX,
        u64::MAX,
        Role::member(i16::MIN),
        [0; 32],
        SecretBox {
            nonce: [0; 16],
            ciphertext: vec![],
        },
        KvDirectoryStatus::Dead,
        [0; 32],
        u64::MAX,
    )
    .map_err(|_| Error::Bounds)?;
    let small = KvSmallFileBox {
        key: RoleAndGeneration {
            role: Role::member(i16::MIN),
            generation: u64::MAX,
        },
        ciphertext: vec![],
    };
    Ok((entry, small))
}
/// Valid directory schema only; malformed input retains decoder-wide limits.
pub fn directory_decode(length: usize) -> Result<usize> {
    if length > crate::directory::MAXIMUM_DIRECTORY_BYTES {
        return Err(Error::Bounds);
    }
    add(&[
        mul(4, length)?,
        mul(60, layout().snowpack_value_bytes)?,
        21_064,
    ])
}
pub fn parent(length: usize) -> Result<usize> {
    let l = layout();
    let decode = directory_decode(length)?;
    let slices = length.div_ceil(crate::directory::DIRECTORY_SLICE_BYTES);
    add(&[
        49,
        mul(3, l.value_bytes)?,
        query(P::DirectoryProbe, 1, 3, 69, &l)?,
        length,
        mul(
            slices,
            add(&[
                49,
                mul(6, l.value_bytes)?,
                query(P::DirectorySlice, 1, 6, 129, &l)?,
            ])?,
        )?,
        decode,
    ])
}
/// Valid normalized List plus the unchanged conservative RPC response hook.
/// Binary payloads use total encoded length; this is cumulative, not live memory.
pub fn frame(length: usize, entries: usize, bodies: usize) -> Result<usize> {
    if length > crate::list_encoder::MAX_LIST_BYTES || bodies > entries || entries > 4096 {
        return Err(Error::Bounds);
    }
    let v = layout().snowpack_value_bytes;
    let (entry, small) = schemas()?;
    let entry = shape(&entry.to_value(), v)?.0;
    let small = shape(&small.to_value(), v)?.0;
    let extended_pair = add(&[mul(8, v)?, 4096, small])?;
    add(&[
        mul(4, length)?,
        mul(mul(4, add(&[3, entries, bodies])?)?, v)?,
        3 * 4096,
        mul(entries, entry)?,
        mul(bodies, extended_pair)?,
        mul(2, add(&[length, 128])?)?,
    ])
}
pub fn bounds() -> Result<Bounds> {
    let l = layout();
    let v = l.snowpack_value_bytes;
    let d = 4096;
    let n = 65536;
    let (entry, small) = schemas()?;
    let (dd, de) = shape(&entry.to_value(), v)?;
    let (sd, se) = shape(&small.to_value(), v)?;
    let decode_admission = add(&[mul(4, add(&[d, n])?)?, 8192])?;
    let decode_schema = add(&[dd, mul(4, d)?, de, d, 4096, sd, mul(4, n)?])?;
    let append_schema = add(&[d, n, 512, mul(40, v)?])?;
    let append_encode = add(&[de, d, 4096, se, n, 4096, 9, 4096])?;
    let page_params = add(&[mul(6, l.value_bytes)?, 97])?;
    let page_query = query(P::ListMacContinue, 15, 6, 137, &l)?;
    let page_decoded = mul(15, l.decoded_bytes)?;
    let page_roles = mul(
        3,
        add(&[
            33,
            mul(7, std::mem::size_of::<Option<u64>>())?,
            query(P::RoleGenerations, 7, 16, 333, &l)?,
        ])?,
    )?;
    let row_decode = add(&[decode_admission, decode_schema])?;
    let row_append = add(&[append_schema, append_encode])?;
    let page_transport = add(&[page_params, page_query, page_decoded])?;
    let full_page_scratch = add(&[
        page_transport,
        page_roles,
        mul(15, add(&[row_decode, row_append])?)?,
    ])?;
    // grow charges full requested replacement capacity. Before a cap-clamped
    // final growth, each requested capacity at least doubles its predecessor,
    // so that geometric sum is less than twice its last term. Each buffer's
    // final requested capacity is <=2*its accepted length. Combined accepted
    // length <=M: without clamping, total charges <4M. At most one buffer can
    // clamp from old capacity c>M/2 to M. Its sum <2c+M; the other buffer sum
    // <4(M-c), yielding <5M-2c<=4M. Actual allocator spare capacity cannot
    // increase this charge sum, because the next requested growth doubles it.
    let retained_growth = mul(4, crate::list_encoder::MAX_LIST_BYTES)?;
    let term = |site, scope, bytes: usize| Term {
        site,
        scope,
        bytes: bytes as u64,
    };
    Ok(Bounds {
        complete: false,
        page_transport: page_transport as u64,
        row_decode: row_decode as u64,
        row_append: row_append as u64,
        page_roles: page_roles as u64,
        full_page_scratch: full_page_scratch as u64,
        retained_growth: retained_growth as u64,
        finish: crate::list_encoder::MAX_LIST_BYTES as u64,
        parent: parent(crate::directory::MAXIMUM_DIRECTORY_BYTES)? as u64,
        frame: frame(crate::list_encoder::MAX_LIST_BYTES, 4096, 4096)? as u64,
        terms: [
            term(
                "parent valid-schema cumulative",
                "parent",
                parent(crate::directory::MAXIMUM_DIRECTORY_BYTES)?,
            ),
            term(
                "valid List validation + RPC response hook",
                "frame",
                frame(crate::list_encoder::MAX_LIST_BYTES, 4096, 4096)?,
            ),
            term("list::parameters", "page", page_params),
            term(
                "query_page bindings/cursor/encoded page",
                "page",
                page_query,
            ),
            term("list Decoded container", "page", page_decoded),
            term("list::decode explicit admission", "row", decode_admission),
            term(
                "Snowpack decode + typed dirent re-encoding",
                "row",
                decode_schema,
            ),
            term("ListEncoder schema copies", "row", append_schema),
            term(
                "ListEncoder entry/body/position encode",
                "row",
                append_encode,
            ),
            term("three seven-role batches", "page", page_roles),
            term(
                "ListEncoder::grow geometric cumulative",
                "invocation",
                retained_growth,
            ),
            term(
                "ListEncoder::finish exact output",
                "invocation",
                crate::list_encoder::MAX_LIST_BYTES,
            ),
        ],
        uncovered: [
            "malformed parent/List: decoder-wide bounds, not valid-schema formulas",
            "actual parent/frame live capacity and fragmentation, separate from charges",
            "scalar aggregate formula pending; S1 requested native copies are admitted",
            "S2 Wasm scalar JSON and TLS buffer lifetime",
            "S3 domain credential/DER/authority copies",
            "SQLite/JS host internals, allocator fragmentation and physical peak",
        ],
    })
}
