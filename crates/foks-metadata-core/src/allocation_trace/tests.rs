use super::*;
use serde_json::json;
fn input() -> serde_json::Value {
    json!({"version":1,"peer":{"exact_der":[48,0],"serial_magnitude":[7],"public_key":vec![4u8;32],"expires_at_micros":"1000"},"now_micros":"200","parent":vec![1u8;16],"cursor":{"kind":"initial"},"count":"4096","load_small":true,"frame":true,"rpc_sequence":"0","fail_at":"0"})
}
fn valid(v: &serde_json::Value) -> bool {
    Invocation::decode(&serde_json::to_vec(v).unwrap()).is_ok()
}
#[test]
fn strict_shape_duplicates_and_full_u64_cursor() {
    let good = input();
    assert!(valid(&good));
    for cursor in [
        json!({"kind":"initial","value":null}),
        json!({"kind":"initial","value":"0"}),
        json!({"kind":"mac"}),
        json!({"kind":"time","value":0}),
        json!({"kind":{"initial":null}}),
        json!({"kind":"mac","value":vec![1u8;33]}),
        json!({"kind":"time","value":"+1"}),
    ] {
        let mut v = good.clone();
        v["cursor"] = cursor;
        assert!(!valid(&v));
    }
    let mut v = good.clone();
    v["cursor"] = json!({"kind":"time","value":u64::MAX.to_string()});
    assert!(valid(&v));
    for field in good.as_object().unwrap().keys() {
        let text = serde_json::to_string(&good).unwrap();
        let prefix = format!(
            "{{{}:{},",
            serde_json::to_string(field).unwrap(),
            good[field]
        );
        let duplicate = prefix + &text[1..];
        assert!(Invocation::decode(duplicate.as_bytes()).is_err(), "{field}");
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(!valid(&v));
    }
    for (old, new) in [
        (
            "\"kind\":\"initial\"",
            "\"kind\":\"initial\",\"kind\":\"mac\"",
        ),
        (
            "\"expires_at_micros\":\"1000\"",
            "\"expires_at_micros\":\"1000\",\"expires_at_micros\":\"999\"",
        ),
    ] {
        let s = serde_json::to_string(&good).unwrap();
        assert!(s.contains(old));
        assert!(Invocation::decode(s.replace(old, new).as_bytes()).is_err());
    }
    let mut v = good;
    v["claimed_uid"] = json!([1, 2]);
    assert!(!valid(&v));
}
#[test]
fn maximum_fixture_representation_has_explicit_rounding_bound() {
    // All-byte255 maximizes compact byte-array spelling; MAC is longer than Time.
    let v = json!({"version":1,"peer":{"exact_der":vec![255u8;4096],"serial_magnitude":vec![255u8;20],"public_key":vec![255u8;32],"expires_at_micros":i64::MAX.to_string()},"now_micros":i64::MAX.to_string(),"parent":vec![255u8;16],"cursor":{"kind":"mac","value":vec![255u8;32]},"count":u64::MAX.to_string(),"load_small":false,"frame":false,"rpc_sequence":u64::MAX.to_string(),"fail_at":"1000000"});
    let bytes = serde_json::to_vec(&v).unwrap();
    assert_eq!(bytes.len(), 17_100);
    assert_eq!(17_100usize.div_ceil(4096) * 4096, MAX_INPUT);
    assert!(Invocation::decode(&bytes).is_ok());
    let mut padded = bytes;
    padded.resize(MAX_INPUT, b' ');
    assert!(Invocation::decode(&padded).is_ok());
    padded.push(b' ');
    assert!(matches!(Invocation::decode(&padded), Err(Error::Bounds)));
    for field in ["exact_der", "serial_magnitude"] {
        let mut bad = v.clone();
        bad["peer"][field].as_array_mut().unwrap().push(json!(255));
        assert!(!valid(&bad));
    }
}
#[test]
fn fixed_recorder_tracks_nested_phases_refusal_and_allocation_failure_separately() {
    let recorder = Arc::new(Recorder::new(3));
    let _active = ActiveGuard(ACTIVE.with(|a| a.replace(Some(recorder.clone()))));
    {
        let _g = phase(Phase::Page);
        assert!(recorder.reserve(10));
        {
            let _g = phase(Phase::Growth);
            assert!(recorder.reserve(20));
        }
        assert!(!recorder.reserve(30));
    }
    let state = *recorder.state.lock().unwrap();
    assert_eq!(state.total.admitted_bytes, 30);
    assert_eq!(state.first_refusal, 3);
    assert!(!state.allocator_failed);
    assert_eq!(state.phases[Phase::Page as usize].refused_calls, 1);
    assert!(foks_rpc::ResponseAllocationBudget::reserve(
        &*recorder, 7, true
    ));
    let state = *recorder.state.lock().unwrap();
    assert_eq!(state.origins[0].admitted_bytes, 30);
    assert_eq!(state.origins[1].admitted_bytes, 7);
    let event = state.prefix[3].unwrap();
    assert!(matches!(event.origin, Origin::RpcResponse));
    assert!(event.control && event.accepted);
    assert_eq!((event.ordinal, event.bytes), (4, 7));
    let actual = Vec::<u8>::new().try_reserve_exact(usize::MAX);
    assert!(actual.is_err());
    allocator_failed();
    assert!(recorder.state.lock().unwrap().allocator_failed);
    assert_eq!(PHASE.with(|p| p.get() as usize), Phase::Other as usize);
}
#[test]
fn bounded_serializer_and_formula_are_not_runtime_allowances() {
    let bytes = encode_report(&layout_report().unwrap()).unwrap();
    assert!(bytes.len() < MAX_OUTPUT);
    assert!(!formula::bounds().unwrap().complete);
    assert!(encode_report(&"x".repeat(MAX_OUTPUT)).is_err());
    let b = formula::bounds().unwrap();
    assert!(b.row_decode > 4 * (4096 + 65536));
    assert!(b.page_transport > 15 * (4096 + 65536));
    assert!(b.page_roles > 0);
    assert_eq!(b.selected_auth_caller, 82_296);
    let caller = b
        .terms
        .iter()
        .find(|term| term.site == "one authenticate/recheck/profile/parent-key caller payload")
        .unwrap();
    assert_eq!(caller.scope, "selected-callgraph cumulative");
    assert_eq!(caller.bytes, b.selected_auth_caller);
}
#[test]
fn valid_schema_formula_matches_codec_charge_and_encoder_capacity_terms() {
    use foks_proto::{
        KvDirectoryStatus, KvDirent, KvNodeId, KvSmallFileBox, Role, RoleAndGeneration, SecretBox,
    };
    for role in [
        Role::NONE,
        Role::ADMIN,
        Role::OWNER,
        Role::member(i16::MIN),
        Role::member(i16::MAX),
    ] {
        let d = KvDirent::new(
            [0; 16],
            [0; 16],
            KvNodeId([3; 17]),
            u64::MAX,
            u64::MAX,
            role,
            [0; 32],
            SecretBox {
                nonce: [0; 16],
                ciphertext: vec![7; 3500],
            },
            KvDirectoryStatus::Dead,
            [0; 32],
            u64::MAX,
        )
        .unwrap();
        let n = KvSmallFileBox {
            key: RoleAndGeneration {
                role,
                generation: u64::MAX,
            },
            ciphertext: vec![8; 65000],
        };
        for v in [d.to_value(), n.to_value()] {
            let exact = foks_snowpack::encode(&v).unwrap();
            let (decode, capacity) =
                formula::shape(&v, std::mem::size_of::<foks_snowpack::Value>()).unwrap();
            let r = Arc::new(Recorder::new(0));
            let decoded =
                foks_snowpack::with_allocation_budget(r.clone(), || foks_snowpack::decode(&exact))
                    .unwrap();
            assert_eq!(r.state.lock().unwrap().total.admitted_bytes, decode as u64);
            let r = Arc::new(Recorder::new(0));
            let encoded = foks_snowpack::with_allocation_budget(r.clone(), || {
                foks_snowpack::encode(&decoded)
            })
            .unwrap();
            assert_eq!(encoded, exact);
            assert_eq!(
                r.state.lock().unwrap().total.admitted_bytes,
                (capacity + 4096) as u64
            );
        }
    }
}

#[test]
fn bounded_prefix_and_complete_page_totals_keep_nested_growth_distinct() {
    let r = Arc::new(Recorder::new(0));
    let _active = ActiveGuard(ACTIVE.with(|a| a.replace(Some(r.clone()))));
    {
        let _page = phase(Phase::Page);
        assert!(r.reserve(10));
        {
            let _decode = phase(Phase::Decode);
            for _ in 0..32 {
                assert!(r.reserve(2));
            }
        }
        {
            let _growth = phase(Phase::Growth);
            assert!(r.reserve(100));
        }
    }
    let s = *r.state.lock().unwrap();
    assert_eq!(
        (s.page_scopes, s.maximum_page_charge, s.maximum_page_scratch),
        (1, 174, 74)
    );
    assert_eq!(s.total.attempts, 34);
    assert_eq!(s.prefix.iter().flatten().count(), PREFIX);
    assert_eq!(s.prefix[15].unwrap().ordinal, 16);
    assert_eq!(PHASE.with(|p| p.get() as usize), Phase::Other as usize);
}
#[test]
fn maximum_diagnostic_shape_is_bounded_before_output_allocation() {
    let totals = Totals {
        attempts: u64::MAX,
        admitted_bytes: u64::MAX,
        refused_calls: u64::MAX,
        first_refusal: u64::MAX,
    };
    let r = Report {
        version: 1,
        outcome: "unexpected_personal_error",
        target: layout(),
        phases: std::array::from_fn(|n| PhaseTotal {
            name: NAMES[n],
            totals,
        }),
        total: totals,
        origins: [
            OriginTotal {
                origin: Origin::Snowpack,
                totals,
            },
            OriginTotal {
                origin: Origin::RpcResponse,
                totals,
            },
        ],
        prefix: [Some(Event {
            ordinal: u64::MAX,
            phase: "parent_authorization",
            origin: Origin::RpcResponse,
            control: true,
            bytes: u64::MAX,
            accepted: false,
        }); PREFIX],
        page_scopes: u64::MAX,
        maximum_page_charge: u64::MAX,
        maximum_page_scratch: u64::MAX,
        workspace_reservations: u64::MAX,
        workspace_bytes: u64::MAX,
        response_bytes: u64::MAX,
        list_sha256: Some("f".repeat(64)),
        frame_bytes: u64::MAX,
        frame_sha256: Some("f".repeat(64)),
        sql_calls: u64::MAX,
        first_refusal: u64::MAX,
        allocator_failed: true,
        memory_pages: Some(u64::MAX),
    };
    assert!(encode_report(&r).unwrap().len() <= MAX_OUTPUT);
}

#[test]
fn only_explicit_physical_framing_failure_sets_allocator_marker() {
    let r = Arc::new(Recorder::new(0));
    let _active = ActiveGuard(ACTIVE.with(|a| a.replace(Some(r.clone()))));
    for kind in [std::io::ErrorKind::WouldBlock, std::io::ErrorKind::Other] {
        assert_eq!(frame_error(foks_rpc::Error::Io(kind.into())), "frame_error");
        assert!(!r.state.lock().unwrap().allocator_failed);
    }
    assert_eq!(
        frame_error(foks_rpc::Error::Io(std::io::ErrorKind::OutOfMemory.into())),
        "allocator_failed"
    );
    assert!(r.state.lock().unwrap().allocator_failed);
}
#[test]
fn valid_parent_and_complete_list_frame_formulas_bound_real_codec_charges() {
    use foks_proto::{
        KvDirectory, KvDirectoryStatus, KvDirent, KvNodeId, KvSmallFileBox, Role,
        RoleAndGeneration, SecretBox,
    };
    use foks_snowpack::Value;
    for role in [
        Role::NONE,
        Role::ADMIN,
        Role::OWNER,
        Role::member(i16::MIN),
        Role::member(i16::MAX),
    ] {
        for length in [0, 31, 524_000] {
            let parent = KvDirectory {
                id: [3; 16],
                version: u64::MAX,
                key: RoleAndGeneration {
                    role,
                    generation: u64::MAX,
                },
                seed_ciphertext: vec![5; length],
                write_role: role,
                status: KvDirectoryStatus::Dead,
            };
            let value = parent.to_value();
            let exact = parent.encode().unwrap();
            let (shape, _) = formula::shape(&value, std::mem::size_of::<Value>()).unwrap();
            let r = Arc::new(Recorder::new(0));
            foks_snowpack::with_allocation_budget(r.clone(), || KvDirectory::decode(&exact))
                .unwrap();
            assert_eq!(r.state.lock().unwrap().total.admitted_bytes, shape as u64);
            assert!(shape <= formula::directory_decode(exact.len()).unwrap());
            if role == Role::member(i16::MIN) {
                assert_eq!(
                    shape,
                    4 * length + 60 * std::mem::size_of::<Value>() + 21_064
                );
            }
        }
        let entry = KvDirent::new(
            [0; 16],
            [1; 16],
            KvNodeId([3; 17]),
            1,
            1,
            role,
            [2; 32],
            SecretBox {
                nonce: [3; 16],
                ciphertext: vec![4; 128],
            },
            KvDirectoryStatus::Active,
            [5; 32],
            0,
        )
        .unwrap();
        let small = KvSmallFileBox {
            key: RoleAndGeneration {
                role,
                generation: 1,
            },
            ciphertext: vec![6; 1024],
        };
        for count in [0, 1, 15, 16, 64] {
            for bodies in [0, count] {
                let value = Value::Array(vec![
                    if count == 0 {
                        Value::Null
                    } else {
                        Value::Array((0..count).map(|_| entry.to_value()).collect())
                    },
                    Value::Bool(false),
                    if bodies == 0 {
                        Value::Null
                    } else {
                        Value::Array(
                            (0..bodies)
                                .map(|i| {
                                    Value::Array(vec![Value::Unsigned(i as u64), small.to_value()])
                                })
                                .collect(),
                        )
                    },
                ]);
                let exact = foks_snowpack::encode(&value).unwrap();
                let length = exact.len();
                let r = Arc::new(Recorder::new(0));
                foks_snowpack::with_allocation_budget(r.clone(), || {
                    foks_rpc::with_response_budget(r.clone(), || {
                        foks_rpc::encode_success_response_owned_at(exact, u64::MAX)
                    })
                })
                .unwrap();
                assert!(
                    r.state.lock().unwrap().total.admitted_bytes
                        <= formula::frame(length, count, bodies).unwrap() as u64
                );
            }
        }
    }
    for n in [
        0,
        1,
        524288,
        524289,
        crate::directory::MAXIMUM_DIRECTORY_BYTES,
    ] {
        assert!(formula::parent(n).unwrap() >= formula::directory_decode(n).unwrap());
    }
    assert!(formula::parent(usize::MAX).is_err());
    assert!(formula::frame(usize::MAX, 1, 1).is_err());
    assert!(formula::frame(1, 4097, 0).is_err());
    assert!(formula::frame(1, 1, 2).is_err());
}

#[test]
fn retained_owner_uses_original_cumulative_recorder_and_survives_output() {
    use crate::list_encoder::owned::{ListRetainedOwner, OwnedListEncoder};
    let recorder = Arc::new(Recorder::new(0));
    let weak = Arc::downgrade(&recorder);
    let raw = {
        let _phase = phase(Phase::Finish);
        OwnedListEncoder::new(0, recorder.clone())
            .unwrap()
            .finish(true)
            .unwrap()
    };
    let initial = recorder.state.lock().unwrap().total;
    assert_eq!(initial.attempts, 1);
    assert_eq!(initial.admitted_bytes, raw.len() as u64);
    assert_eq!(
        recorder.state.lock().unwrap().phases[Phase::Finish as usize].attempts,
        1
    );
    assert!(recorder.reserve_retained(10));
    assert_eq!(
        recorder.state.lock().unwrap().total.admitted_bytes,
        initial.admitted_bytes + 10
    );
    drop(recorder);
    assert!(weak.upgrade().is_some());
    drop(raw);
    assert!(weak.upgrade().is_none());
    let recorder = Arc::new(Recorder::new(1));
    assert!(OwnedListEncoder::new(0, recorder.clone())
        .unwrap()
        .finish(true)
        .is_err());
    assert!(recorder.failed());
    let state = recorder.state.lock().unwrap();
    assert_eq!(state.first_refusal, 1);
    assert!(!state.allocator_failed);
}
