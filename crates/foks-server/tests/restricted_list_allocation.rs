//! Requested allocation evidence for the actual restricted List parsers.
//! Inputs, owners and instrumentation are prepared outside each measured call.
//! Realloc counts old+new coexistence, even when System can grow in place. This
//! measures requested Rust allocation bytes, never SQLite/TLS memory or RSS.
#![cfg(all(unix, feature = "celld-proof"))]

use foks_schema_contract::{Contract, ContractObject, MatchPolicy, SchemaObject};
use foks_server::restricted_storage::CertificateParserQualification;
use foks_snowpack::{AllocationBudget, PathSegment};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::RefCell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Clone, Copy)]
struct Slot {
    address: usize,
    bytes: usize,
}
const EMPTY: Slot = Slot {
    address: 0,
    bytes: 0,
};
struct Tracking {
    enabled: bool,
    slots: [Slot; 4096],
    live: usize,
    peak: usize,
    traffic: usize,
    invalid: bool,
}
impl Tracking {
    const fn new() -> Self {
        Self {
            enabled: false,
            slots: [EMPTY; 4096],
            live: 0,
            peak: 0,
            traffic: 0,
            invalid: false,
        }
    }
    fn allocated(&mut self, pointer: *mut u8, bytes: usize) {
        if !self.enabled || pointer.is_null() {
            return;
        }
        let Some(live) = self.live.checked_add(bytes) else {
            self.invalid = true;
            return;
        };
        let Some(traffic) = self.traffic.checked_add(bytes) else {
            self.invalid = true;
            return;
        };
        let Some(slot) = self.slots.iter_mut().find(|s| s.address == 0) else {
            self.invalid = true;
            return;
        };
        *slot = Slot {
            address: pointer as usize,
            bytes,
        };
        self.live = live;
        self.traffic = traffic;
        self.peak = self.peak.max(live);
    }
    fn freed(&mut self, pointer: *mut u8) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.address == pointer as usize)
        {
            if let Some(live) = self.live.checked_sub(slot.bytes) {
                self.live = live;
            } else {
                self.invalid = true;
            }
            *slot = EMPTY;
        }
    }
    fn reallocating(&mut self, pointer: *mut u8, bytes: usize) {
        if !self.enabled {
            return;
        }
        // An input allocation must never be resized inside these measurements.
        // Refuse to certify an interval with an untracked old allocation.
        if !self.slots.iter().any(|s| s.address == pointer as usize) {
            self.invalid = true;
        }
        if let Some(peak) = self.live.checked_add(bytes) {
            self.peak = self.peak.max(peak);
        } else {
            self.invalid = true;
        }
    }
}
thread_local! { static TRACKING: RefCell<Tracking> = const { RefCell::new(Tracking::new()) }; }
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc_zeroed(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = TRACKING.try_with(|s| s.borrow_mut().freed(pointer));
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        let _ = TRACKING.try_with(|s| s.borrow_mut().reallocating(pointer, bytes));
        let result = unsafe { System.realloc(pointer, layout, bytes) };
        if !result.is_null() {
            let _ = TRACKING.try_with(|s| {
                let mut s = s.borrow_mut();
                s.freed(pointer);
                s.allocated(result, bytes);
            });
        }
        result
    }
}
struct Measurement;
impl Measurement {
    fn begin() -> Self {
        let enabled = TRACKING.with(|s| s.borrow().enabled);
        assert!(!enabled);
        TRACKING.with(|s| {
            let mut s = s.borrow_mut();
            *s = Tracking::new();
            s.enabled = true;
        });
        Self
    }
}
impl Drop for Measurement {
    fn drop(&mut self) {
        TRACKING.with(|s| s.borrow_mut().enabled = false);
    }
}
#[derive(Clone, Copy, Debug)]
struct Observed {
    peak: usize,
    traffic: usize,
}
fn observed() -> Observed {
    let (enabled, invalid, live, peak, traffic) = TRACKING.with(|s| {
        let s = s.borrow();
        (s.enabled, s.invalid, s.live, s.peak, s.traffic)
    });
    // Failed assertions format/allocate: release the tracker's borrow first.
    assert!(!enabled);
    assert!(!invalid, "allocation tracker lost an owner");
    assert_eq!(live, 0, "a measured allocation escaped the interval");
    Observed { peak, traffic }
}

#[test]
fn tracker_counts_realloc_old_and_new_and_requires_physical_drop() {
    let measure = Measurement::begin();
    let mut bytes = Vec::with_capacity(8);
    bytes.extend([0u8; 8]);
    bytes.extend([1u8; 8]);
    std::hint::black_box(&bytes);
    drop(bytes);
    drop(measure);
    let used = observed();
    assert_eq!(used.traffic, 24);
    assert_eq!(used.peak, 24);
}

// Deterministic DER constructors. These fixtures exercise extraction only and
// intentionally do not assert a signature, TLS possession or current SQL owner.
fn tlv(tag: u8, bytes: &[u8]) -> Vec<u8> {
    assert!(bytes.len() <= u16::MAX as usize);
    let mut out = vec![tag];
    if bytes.len() < 128 {
        out.push(bytes.len() as u8);
    } else if bytes.len() <= 255 {
        out.extend([0x81, bytes.len() as u8]);
    } else {
        out.push(0x82);
        out.extend((bytes.len() as u16).to_be_bytes());
    }
    out.extend(bytes);
    out
}
fn join(tag: u8, parts: &[&[u8]]) -> Vec<u8> {
    tlv(tag, &parts.concat())
}
fn name(attributes: usize, separate_rdns: bool, malformed_tail: bool) -> Vec<u8> {
    let attribute = join(0x30, &[&tlv(0x06, &[0x2a]), &tlv(0x0c, b"")]);
    let mut content = Vec::new();
    for _ in 0..attributes {
        if separate_rdns {
            content.extend(tlv(0x31, &attribute));
        } else {
            content.extend(&attribute);
        }
    }
    if malformed_tail {
        content.extend([0x30, 0x04, 0x06, 0x01, 0x2a]);
    }
    if !separate_rdns && !content.is_empty() {
        content = tlv(0x31, &content);
    }
    tlv(0x30, &content)
}
fn extensions(count: usize, malformed_tail: bool) -> Vec<u8> {
    let extension = join(
        0x30,
        &[&tlv(0x06, &[0x55, 0x1d, 0x13]), &tlv(0x04, &[0x30, 0])],
    );
    let mut bytes = extension.repeat(count);
    if malformed_tail {
        bytes.extend([0x30, 0x04, 0x06, 0x01, 0x2a]);
    }
    tlv(0xa3, &tlv(0x30, &bytes))
}
fn certificate(
    serial: &[u8],
    issuer: &[u8],
    subject: &[u8],
    extra: &[u8],
    oid: &[u8],
    times: &[u8],
) -> Vec<u8> {
    let algorithm = tlv(0x30, &tlv(0x06, oid));
    let mut key = vec![0];
    key.extend([0x11; 32]);
    let spki = join(0x30, &[&algorithm, &tlv(0x03, &key)]);
    let tbs = join(
        0x30,
        &[
            &tlv(0xa0, &tlv(0x02, &[2])),
            &tlv(0x02, serial),
            &algorithm,
            issuer,
            times,
            subject,
            &spki,
            extra,
        ],
    );
    join(0x30, &[&tbs, &algorithm, &tlv(0x03, &[0; 65])])
}
fn times(before: &[u8], after: &[u8]) -> Vec<u8> {
    join(0x30, &[&tlv(0x17, before), &tlv(0x17, after)])
}
fn parser_case(der: &[u8], success: Option<bool>) {
    assert!(!der.is_empty() && der.len() <= 4096);
    let mut parser = CertificateParserQualification::new(der.len()).unwrap();
    let allowance = CertificateParserQualification::allowance(der.len()).unwrap();
    let length = der.len();
    let expected =
        4 * (length / 6) * std::mem::size_of::<x509_parser::x509::AttributeTypeAndValue<'static>>()
            + 4 * (length / 8 + 2)
                * std::mem::size_of::<x509_parser::x509::RelativeDistinguishedName<'static>>()
            + 4 * (length / 6 + 1)
                * std::mem::size_of::<x509_parser::extensions::X509Extension<'static>>()
            + length
            + 2 * ((length + 7) & !7)
            + 2 * 53;
    assert_eq!(allowance, expected);
    let measure = Measurement::begin();
    let result = parser.parse(der);
    drop(measure);
    let used = observed();
    if let Some(success) = success {
        assert_eq!(result.is_ok(), success, "DER length {}", der.len());
    }
    assert!(
        used.traffic <= allowance,
        "parser cumulative traffic {used:?} exceeds {allowance}"
    );
    assert!(
        used.peak <= allowance,
        "parser realloc overlap {used:?} exceeds {allowance}"
    );
    // Reuse is legal only after the complete parser tree has dropped; the
    // measurement above requires zero live allocations even on malformed input.
    let measure = Measurement::begin();
    let repeated = parser.parse(der);
    drop(measure);
    assert_eq!(result, repeated);
    assert_eq!(used.traffic, observed().traffic);
}

#[test]
fn actual_certificate_parser_allocation_fits_the_preadmitted_workspace() {
    let empty = name(0, false, false);
    let valid_times = times(b"240101000000Z", b"490101000000Z");
    let oid = [0x2b, 0x65, 0x70];
    let base = certificate(&[1], &empty, &empty, &[], &oid, &valid_times);
    parser_case(&base, Some(true));
    let mut parser = CertificateParserQualification::new(base.len()).unwrap();
    let fields = parser.parse(&base).unwrap();
    assert_eq!(fields.public_key, [0x11; 32]);
    assert_eq!(&fields.serial[..usize::from(fields.serial_length)], &[1]);
    // Boundaries straddle all relevant nom/Vec growth steps and put failed
    // partially allocated children after already accepted names/extensions.
    for count in [1, 4, 5, 8, 9, 16, 17, 32, 33, 64, 65, 96, 129] {
        for separate in [false, true] {
            let valid = name(count, separate, false);
            parser_case(
                &certificate(&[1], &valid, &valid, &[], &oid, &valid_times),
                Some(true),
            );
            let malformed = name(count, separate, true);
            parser_case(
                &certificate(&[1], &valid, &malformed, &[], &oid, &valid_times),
                None,
            );
        }
        parser_case(
            &certificate(
                &[1],
                &empty,
                &empty,
                &extensions(count, false),
                &oid,
                &valid_times,
            ),
            Some(true),
        );
        parser_case(
            &certificate(
                &[1],
                &empty,
                &empty,
                &extensions(count, true),
                &oid,
                &valid_times,
            ),
            None,
        );
    }
    for length in [1, 8, 9, 20, 21, 64, 65, 255, 256, 1024, 2048, 3500] {
        let mut serial = vec![0; length];
        serial[length - 1] = 1;
        parser_case(
            &certificate(&serial, &empty, &empty, &[], &oid, &valid_times),
            Some(true),
        );
    }
    for bad_oid in [vec![0x2b, 0x65, 0x71], vec![0x80; 512], vec![0x2a; 1024]] {
        parser_case(
            &certificate(&[1], &empty, &empty, &[], &bad_oid, &valid_times),
            Some(false),
        );
    }
    let large = name(100, true, false);
    for invalid in [b"991399000000Z".as_slice(), b"not-a-time!!!", b""] {
        for validity in [
            times(invalid, b"490101000000Z"),
            times(b"240101000000Z", invalid),
        ] {
            parser_case(
                &certificate(&[1], &large, &empty, &[], &oid, &validity),
                Some(false),
            );
        }
    }
    // GeneralizedTime has the longest reachable static time-error message
    // (53 bytes), separately from UTCTime's shorter formatting failures.
    let invalid_generalized = tlv(0x18, b"20240101000000.X");
    for validity in [
        join(0x30, &[&invalid_generalized, &tlv(0x17, b"490101000000Z")]),
        join(0x30, &[&tlv(0x17, b"240101000000Z"), &invalid_generalized]),
    ] {
        parser_case(
            &certificate(&[1], &large, &empty, &[], &oid, &validity),
            Some(false),
        );
    }
    let full = certificate(
        &[1],
        &large,
        &large,
        &extensions(100, false),
        &oid,
        &valid_times,
    );
    for length in 1..full.len() {
        parser_case(&full[..length], Some(false));
    }
}

#[derive(Default)]
struct PayloadBudget {
    reserved: AtomicUsize,
}
impl AllocationBudget for PayloadBudget {
    fn reserve(&self, bytes: usize) -> bool {
        self.reserved.fetch_add(bytes, Ordering::Relaxed);
        true
    }
}
fn diagnostic_headroom() -> usize {
    // The reviewed path/error/formatting formula uses the actual native enum
    // layout. Pin that layout independently of the production helper.
    assert_eq!(std::mem::size_of::<PathSegment>(), 24);
    assert_eq!(std::mem::align_of::<PathSegment>(), 8);
    let overhead = (128 + 65) * std::mem::size_of::<PathSegment>()
        + 2 * 65 * 31
        + (3 * 6179 + 8)
        + (9 * 31 + 8);
    assert_eq!(overhead, 27_494);
    overhead
}
fn decoder_case(bytes: &[u8]) {
    assert!(bytes.len() <= 16_384);
    let overhead = diagnostic_headroom();
    for list in [false, true] {
        let budget = Arc::new(PayloadBudget::default());
        let measure = Measurement::begin();
        let accepted = foks_snowpack::with_allocation_budget(budget.clone(), || {
            if list {
                foks_rpc::arguments::decode_kv_list_arguments(bytes).is_ok()
            } else {
                foks_rpc::arguments::KvAuthorityArgument::decode(bytes).is_ok()
            }
        });
        drop(measure);
        let used = observed();
        let bound = overhead + budget.reserved.load(Ordering::Relaxed);
        assert!(
            used.peak <= bound,
            "decoder accepted={accepted}, list={list}, {used:?}, bound={bound}"
        );
    }
}

#[test]
fn real_f0_and_list_decoders_fit_payload_hooks_plus_diagnostic_headroom() {
    for depth in [0, 1, 4, 8, 16, 32, 63, 64, 65] {
        for tag in [b"x".as_slice(), &[0xff; 31]] {
            let mut bytes = Vec::new();
            for _ in 0..depth {
                bytes.extend([0x81, 0xa0 | tag.len() as u8]);
                bytes.extend(tag);
            }
            for suffix in [
                &[0xca][..],
                &[0xc1],
                &[0xc5, 0x01, 0x00],
                &[0xd0, 0],
                &[0],
                &[],
            ] {
                let mut input = bytes.clone();
                input.extend(suffix);
                decoder_case(&input);
            }
        }
        let mut arrays = vec![0x91; depth];
        arrays.push(0xca);
        decoder_case(&arrays);
    }
    // Offset/index formatting, wide arrays, early EOF paths and fixed
    // schema errors are exercised through both real public decoders.
    let mut wide = vec![0xdc, 0x3f, 0xfd];
    wide.extend([0; 16_380]);
    wide.push(0xca);
    decoder_case(&wide);
    for suffix in [0, 0x80, 0xc0, 0xc1] {
        let mut bytes = vec![0x91, 0x92, 0, 0x80];
        bytes.push(suffix);
        decoder_case(&bytes);
    }
    let valid = foks_snowpack::encode(&foks_snowpack::Value::Array(vec![
        foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(0),
            foks_snowpack::Value::Variant(None),
        ]),
        foks_snowpack::Value::Binary(vec![1; 16]),
        foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Array(vec![
                foks_snowpack::Value::Unsigned(0),
                foks_snowpack::Value::Variant(None),
            ]),
            foks_snowpack::Value::Unsigned(1),
            foks_snowpack::Value::Bool(false),
        ]),
    ]))
    .unwrap();
    decoder_case(&valid);
    assert!(foks_rpc::arguments::KvAuthorityArgument::decode(&valid).is_ok());
    assert!(foks_rpc::arguments::decode_kv_list_arguments(&valid).is_ok());
}

fn ingress_case(argument: &[u8], valid: bool) {
    let overhead = diagnostic_headroom() + 3 * (20 - 5);
    assert_eq!(overhead, 27_539);
    // Unframed CALL_V2 with a headerless protocol reaches the real generic
    // Snowpack validator, before routing or the restricted 16 KiB method cap.
    let mut frame = vec![0x95, 5, 1];
    frame.extend(
        foks_snowpack::encode(&foks_snowpack::Value::Unsigned(foks_rpc::KEX_PROTOCOL_ID)).unwrap(),
    );
    frame.push(1);
    frame.extend(argument);
    let budget = Arc::new(PayloadBudget::default());
    let measure = Measurement::begin();
    let outcome = foks_snowpack::with_allocation_budget(budget.clone(), || {
        // Consume the owned argument or diagnostic inside the interval without
        // formatting the outer RPC error, as the private listener does.
        match foks_rpc::decode_message(&frame) {
            Ok(foks_rpc::InboundMessage::Call(_)) => 0,
            Err(foks_rpc::Error::ArgumentSnowpack { .. }) => 1,
            _ => 2,
        }
    });
    drop(measure);
    let used = observed();
    assert_eq!(outcome, if valid { 0 } else { 1 });
    let bound = 2 * frame.len() + budget.reserved.load(Ordering::Relaxed) + overhead;
    assert!(
        used.peak <= bound,
        "ingress valid={valid}, length={}, {used:?}, bound={bound}",
        frame.len()
    );
}

#[test]
fn actual_rpc_ingress_decoder_fits_input_hooks_and_wide_offset_diagnostics() {
    for depth in [0, 1, 8, 32, 60] {
        let mut argument = Vec::new();
        for _ in 0..depth {
            argument.extend([0x81, 0xbf]);
            argument.extend([b'x'; 31]);
        }
        // Legal MessagePack, invalid canonical Snowpack. This must reach
        // ArgumentSnowpack, rather than pass via an earlier envelope failure.
        argument.extend([0xd0, 0]);
        ingress_case(&argument, false);
    }
    let mut late_error = vec![0x92, 0xc6];
    late_error.extend(100_000u32.to_be_bytes());
    late_error.extend([0; 100_000]);
    late_error.extend([0xd0, 0]);
    ingress_case(&late_error, false);
    ingress_case(&[0xc0], true);
    ingress_case(&[0x90], true);
}

#[test]
fn schema_verifier_string_growth_and_realloc_overlap_fit_2080_bytes() {
    assert!(include_str!("../../../rust-toolchain.toml").contains("channel = \"1.95.0\""));
    for sizes in [
        &[1, 8, 9, 16, 17, 32, 33, 64, 65, 128, 129, 256][..],
        &[63, 64, 127, 253, 256],
        &[255, 256],
        &[256],
    ] {
        let names = sizes.iter().map(|n| "x".repeat(*n)).collect::<Vec<_>>();
        let objects = names
            .iter()
            .map(|name| SchemaObject {
                kind: "table",
                name,
                table: name,
                sql: "CREATE TABLE t(v)",
            })
            .collect::<Vec<_>>();
        let contract = Contract::new(
            objects
                .iter()
                .copied()
                .map(ContractObject::borrowed)
                .collect(),
        )
        .unwrap();
        let measure = Measurement::begin();
        let mut verifier = contract.verifier(MatchPolicy::ExactUnion);
        for object in &objects {
            verifier.observe(*object).unwrap();
        }
        verifier.finish().unwrap();
        drop(measure);
        let used = observed();
        assert!(
            used.peak <= 2080,
            "schema verifier realloc overlap {used:?}"
        );
        assert!(
            used.traffic <= 2080,
            "schema verifier cumulative traffic {used:?}"
        );
    }
}
