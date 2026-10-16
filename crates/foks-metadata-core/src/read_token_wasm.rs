//! Host-owned setup, never a public/frontend request operation. No SQL imports.
use crate::read_token::ReadTokenAuthority;
use std::cell::RefCell;
use zeroize::{Zeroize, Zeroizing};
thread_local! {
    static AUTHORITY:RefCell<Option<ReadTokenAuthority>>=const{RefCell::new(None)};
    static SETUP:RefCell<Zeroizing<Vec<u8>>>=RefCell::new(Zeroizing::new(Vec::new()));
}
#[no_mangle]
pub extern "C" fn read_token_clear() {
    AUTHORITY.with(|value| {
        value.borrow_mut().take();
    });
    SETUP.with(|value| value.borrow_mut().zeroize());
    super::INPUT.with(|value| value.borrow_mut().zeroize());
    super::RESPONSE.with(|value| value.borrow_mut().zeroize());
    super::PAGE_RESPONSE.with(|value| value.borrow_mut().zeroize());
    super::OUTPUT.with(|value| value.borrow_mut().zeroize());
}
#[no_mangle]
pub extern "C" fn read_token_setup_alloc(length: u32) -> u32 {
    read_token_clear();
    if length != 48 {
        return 0;
    }
    SETUP.with(|value| {
        let mut bytes = value.borrow_mut();
        *bytes = Zeroizing::new(vec![0; 48]);
        bytes.as_mut_ptr() as u32
    })
}
#[no_mangle]
pub extern "C" fn read_token_setup() -> u32 {
    AUTHORITY.with(|value| {
        value.borrow_mut().take();
    });
    let mut bytes =
        SETUP.with(|value| std::mem::replace(&mut *value.borrow_mut(), Zeroizing::new(Vec::new())));
    if bytes.len() != 48 {
        return 1;
    }
    let mut entropy = Zeroizing::new([0; 48]);
    entropy.copy_from_slice(&bytes);
    bytes.zeroize();
    match ReadTokenAuthority::from_host_entropy(entropy) {
        Ok(authority) => {
            AUTHORITY.with(|value| *value.borrow_mut() = Some(authority));
            0
        }
        Err(_) => 1,
    }
}

// This selector exists only in the explicitly opted-in fixture artifact. Future
// SQL Begin owns the internal issuer; frontend requests cannot select this path.
#[cfg(feature = "read-token-qualification")]
mod fixture {
    use super::*;
    use crate::{
        authority::{Fence, Number},
        page,
        personal::PeerCertificate,
        read_token::{Error, InitialCursor, ListBinding, Position, ReadDependency},
    };
    use foks_proto::{Role, RoleAndGeneration};
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Dependency {
        incarnation: [u8; 32],
        revision: Number,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        fence: Fence,
        revision: Number,
        dependency: Dependency,
        peer: PeerCertificate,
        namespace: Vec<u8>,
        parent: [u8; 16],
        cursor: InitialCursor,
        requested_count: Number,
        load_small: bool,
        deadline: Number,
        clock_domain: [u8; 32],
    }
    impl Binding {
        fn checked(self) -> Result<ListBinding, Error> {
            ListBinding::new(
                self.fence,
                self.revision,
                ReadDependency::new(
                    self.dependency.incarnation,
                    u64::try_from(self.dependency.revision.0).map_err(|_| Error::InvalidBinding)?,
                )?,
                self.peer,
                self.namespace
                    .try_into()
                    .map_err(|_| Error::InvalidBinding)?,
                self.parent,
                self.cursor,
                u64::try_from(self.requested_count.0).map_err(|_| Error::InvalidBinding)?,
                self.load_small,
                self.deadline,
                self.clock_domain,
            )
        }
    }
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Last {
        None {},
        Mac { value: [u8; 32], id: [u8; 16] },
        Time { value: Number, id: [u8; 16] },
    }
    impl Last {
        fn checked(self) -> Result<Position, Error> {
            Ok(match self {
                Self::None {} => Position::None,
                Self::Mac { value, id } => Position::Mac(value, id),
                Self::Time { value, id } => {
                    Position::Time(u64::try_from(value.0).map_err(|_| Error::InvalidToken)?, id)
                }
            })
        }
    }
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Request {
        Issue {
            binding: Binding,
            role: Number,
            visibility: i32,
            generation: Number,
            bundle: [u8; 16],
            now: Number,
        },
        Verify {
            binding: Binding,
            token: Vec<u8>,
            bundle: [u8; 16],
            ordinal: u16,
            now: Number,
        },
        Advance {
            binding: Binding,
            token: Vec<u8>,
            consumed: u16,
            limit: u16,
            last: Last,
            now: Number,
        },
    }
    fn run(input: &[u8]) -> Result<Vec<u8>, Error> {
        let request: Request = serde_json::from_slice(input).map_err(|_| Error::InvalidBinding)?;
        AUTHORITY.with(|holder| {
            let holder = holder.borrow();
            let authority = holder.as_ref().ok_or(Error::BundleLost)?;
            let result = match request {
                Request::Issue {
                    binding,
                    role,
                    visibility,
                    generation,
                    bundle,
                    now,
                } => {
                    let role = match (role.0, visibility) {
                        (1, v) => Role::member(i16::try_from(v).map_err(|_| Error::InvalidToken)?),
                        (2, 0) => Role::ADMIN,
                        (3, 0) => Role::OWNER,
                        _ => return Err(Error::InvalidToken),
                    };
                    let generation =
                        u64::try_from(generation.0).map_err(|_| Error::InvalidToken)?;
                    authority
                        .issue(
                            &binding.checked()?,
                            RoleAndGeneration { role, generation },
                            bundle,
                            now,
                        )?
                        .as_bytes()
                        .to_vec()
                }
                Request::Verify {
                    binding,
                    token,
                    bundle,
                    ordinal,
                    now,
                } => {
                    let verified = authority.verify(&token, &binding.checked()?, now)?;
                    verified.check_expected(bundle, ordinal)?;
                    vec![
                        u8::from(verified.complete()),
                        u8::from(verified.final_page()),
                    ]
                }
                Request::Advance {
                    binding,
                    token,
                    consumed,
                    limit,
                    last,
                    now,
                } => {
                    let binding = binding.checked()?;
                    let verified = authority.verify(&token, &binding, now)?;
                    authority
                        .advance(&verified, &binding, consumed, limit, last.checked()?, now)?
                        .as_bytes()
                        .to_vec()
                }
            };
            Ok(result)
        })
    }
    #[no_mangle]
    pub extern "C" fn read_token_fixture_alloc(length: u32) -> u32 {
        super::super::INPUT.with(|value| value.borrow_mut().zeroize());
        super::super::OUTPUT.with(|value| value.borrow_mut().zeroize());
        if length == 0 || length > 32768 || page::charge(length as usize).is_err() {
            return 0;
        }
        super::super::input_alloc(length)
    }
    #[no_mangle]
    pub extern "C" fn run_read_token_fixture() -> u32 {
        let input = super::super::INPUT.with(|value| std::mem::take(&mut *value.borrow_mut()));
        let (status, bytes) = match run(&input) {
            Ok(bytes) => (0, bytes),
            Err(error) => (1, format!("{error:?}").into_bytes()),
        };
        super::super::OUTPUT.with(|value| *value.borrow_mut() = bytes);
        status
    }
}
