//! Minimal audited ABI; buffers are owned initialized Vecs, never constructed
//! from foreign pointers. JS may access their published memory ranges only
//! synchronously while an exported call/import is active.
use crate::{wire, Error, Result, Sql, Statement, Value, MAX_INPUT_BYTES, MAX_SQL_CALLS};
use std::cell::RefCell;

thread_local! {
    static INPUT: RefCell<Vec<u8>>=const {RefCell::new(Vec::new())};
    static RESPONSE: RefCell<Vec<u8>>=const {RefCell::new(Vec::new())};
    static OUTPUT: RefCell<Vec<u8>>=const {RefCell::new(Vec::new())};
    static PAGE_RESPONSE: RefCell<Vec<u8>>=const {RefCell::new(Vec::new())};
}

#[link(wasm_import_module = "foks")]
extern "C" {
    fn sql(statement: u32, parameters: *const u8, length: u32, write: u32) -> u32;
    fn domain(input: *const u8, length: u32) -> u32;
    fn sql_page(statement: u32, parameters: *const u8, length: u32) -> u32;
}

/// Private decoder qualification only: no caller authentication or service route.
#[no_mangle]
pub extern "C" fn run_directory_key() -> u32 {
    let input = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = crate::directory::qualify(&mut Host { calls: 0 }, input);
    let (status, bytes) = match result {
        Ok(bytes) => (0, bytes),
        Err(error) => (
            1,
            crate::directory::qualification_error(&error)
                .as_bytes()
                .to_vec(),
        ),
    };
    OUTPUT.with(|b| *b.borrow_mut() = bytes);
    status
}

#[no_mangle]
pub extern "C" fn page_abi_version() -> u32 {
    u32::from(crate::page::ABI_VERSION)
}

#[no_mangle]
pub extern "C" fn page_catalog() -> u32 {
    match crate::page::catalog_artifact() {
        Ok(bytes) => {
            OUTPUT.with(|b| *b.borrow_mut() = bytes);
            0
        }
        Err(_) => {
            OUTPUT.with(|b| b.borrow_mut().clear());
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn page_input_alloc(length: u32) -> u32 {
    INPUT.with(|b| b.borrow_mut().clear());
    OUTPUT.with(|b| b.borrow_mut().clear());
    PAGE_RESPONSE.with(|b| b.borrow_mut().clear());
    if length < crate::page::HEADER_BYTES as u32
        || length as usize > crate::page::MAX_PARAMETER_BYTES
    {
        return 0;
    }
    if crate::page::charge(length as usize).is_err() {
        return 0;
    }
    input_alloc(length)
}

#[no_mangle]
pub extern "C" fn page_response_alloc(length: u32) -> u32 {
    PAGE_RESPONSE.with(|b| b.borrow_mut().clear());
    if length < crate::page::HEADER_BYTES as u32 || length as usize > crate::page::MAX_PAGE_BYTES {
        return 0;
    }
    if crate::page::charge(length as usize).is_err() {
        return 0;
    }
    PAGE_RESPONSE.with(|b| {
        let mut bytes = b.borrow_mut();
        *bytes = vec![0; length as usize];
        bytes.as_mut_ptr() as u32
    })
}

/// Private fixture entry only. It has no personal-authority or production route.
#[no_mangle]
pub extern "C" fn run_page() -> u32 {
    let input = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = crate::page::decode_parameters(input)
        .and_then(|(statement, parameters)| Host { calls: 0 }.query_page(statement, &parameters));
    let (status, bytes) = match result {
        Ok(page) => (0, page.into_bytes()),
        Err(error) => (1, error.code().as_bytes().to_vec()),
    };
    OUTPUT.with(|b| *b.borrow_mut() = bytes);
    status
}

#[no_mangle]
pub extern "C" fn input_alloc(length: u32) -> u32 {
    if length as usize > MAX_INPUT_BYTES {
        return 0;
    }
    INPUT.with(|buffer| {
        let mut b = buffer.borrow_mut();
        *b = vec![0; length as usize];
        b.as_mut_ptr() as u32
    })
}
#[no_mangle]
pub extern "C" fn response_alloc(length: u32) -> u32 {
    // A single row / change-count response is much smaller than a command.
    if length > 64 * 1024 {
        return 0;
    }
    RESPONSE.with(|buffer| {
        let mut b = buffer.borrow_mut();
        *b = vec![0; length as usize];
        b.as_mut_ptr() as u32
    })
}
#[no_mangle]
pub extern "C" fn output_pointer() -> u32 {
    OUTPUT.with(|b| b.borrow().as_ptr() as u32)
}
#[no_mangle]
pub extern "C" fn output_length() -> u32 {
    OUTPUT.with(|b| b.borrow().len() as u32)
}

/// Pure exact-box decoder ABI for the private native-backed replay proof.
/// Rejected allocation invalidates the preceding request and result.
#[no_mangle]
pub extern "C" fn small_box_input_alloc(length: u32) -> u32 {
    INPUT.with(|b| b.borrow_mut().clear());
    OUTPUT.with(|b| b.borrow_mut().clear());
    if length == 0 || length > 65_536 {
        return 0;
    }
    input_alloc(length)
}

/// Does not call a SQL/domain import. Successful output is exact ASCII decimal,
/// never a JS Number; failures cannot become durable domain rejections.
#[no_mangle]
pub extern "C" fn small_box_ciphertext_length() -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = if bytes.is_empty() || bytes.len() > 65_536 {
        Err("input_bound")
    } else {
        foks_proto::KvSmallFileBox::ciphertext_length(&bytes)
            .map_err(|_| "invalid_box")
            .and_then(|length| i64::try_from(length).map_err(|_| "integer_range"))
    };
    let (status, output) = match result {
        Ok(length) => (0, length.to_string().into_bytes()),
        Err(code) => (1, code.as_bytes().to_vec()),
    };
    OUTPUT.with(|b| *b.borrow_mut() = output);
    status
}
#[no_mangle]
pub extern "C" fn trap() {
    core::arch::wasm32::unreachable()
}

struct Host {
    calls: usize,
}
impl Host {
    fn invoke<T: serde::de::DeserializeOwned>(
        &mut self,
        statement: Statement,
        parameters: &[Value],
        write: bool,
    ) -> Result<T> {
        statement.validate_parameters(parameters)?;
        self.calls += 1;
        if self.calls > MAX_SQL_CALLS || statement.spec().write != write {
            return Err(Error::Bounds);
        }
        let encoded = serde_json::to_vec(parameters).map_err(|_| Error::Bounds)?;
        RESPONSE.with(|b| b.borrow_mut().clear());
        // SAFETY: the imported host reads this live immutable slice during this
        // synchronous call only. It writes solely to the buffer allocated by
        // response_alloc; no Rust reference to that buffer is held across SQL.
        let failed = unsafe {
            sql(
                statement as u32,
                encoded.as_ptr(),
                encoded.len() as u32,
                u32::from(write),
            )
        };
        if failed != 0 {
            return Err(Error::Storage);
        }
        let response = RESPONSE.with(|b| std::mem::take(&mut *b.borrow_mut()));
        serde_json::from_slice(&response).map_err(|_| Error::Storage)
    }
}
impl Sql for Host {
    fn query_page(
        &mut self,
        statement: crate::page_catalog::PageStatement,
        parameters: &[Value],
    ) -> Result<crate::page::PageBuffer> {
        let spec = statement.spec();
        let encoded = crate::page::encode_parameters(&spec, parameters)?;
        self.calls += 1;
        if self.calls > MAX_SQL_CALLS {
            return Err(Error::Bounds);
        }
        PAGE_RESPONSE.with(|b| b.borrow_mut().clear());
        // SAFETY: immutable owned parameters remain live through the synchronous
        // import. No RefCell borrow crosses it. The host writes only to its
        // separately allocated PAGE_RESPONSE and reacquires views after growth.
        let failed = unsafe { sql_page(statement as u32, encoded.as_ptr(), encoded.len() as u32) };
        if failed != 0 {
            return Err(Error::Storage);
        }
        let bytes = PAGE_RESPONSE.with(|b| std::mem::take(&mut *b.borrow_mut()));
        let page = crate::page::PageBuffer::decode(bytes, &spec, crate::page::EnvelopeKind::Rows)?;
        if page.len() > usize::from(spec.requested_rows(parameters)?) {
            return Err(Error::Bounds);
        }
        Ok(page)
    }
    fn query(&mut self, s: Statement, p: &[Value]) -> Result<Option<Vec<Value>>> {
        self.invoke(s, p, false)
    }
    fn execute(&mut self, s: Statement, p: &[Value]) -> Result<u64> {
        let value: Value = self.invoke(s, p, true)?;
        crate::unsigned(&value)
    }
    fn domain(&mut self, input: &[u8]) -> Result<wire::Reply> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields)]
        enum DomainResult {
            Success(wire::Reply),
            Rejected(Rejection),
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Rejection {
            Conflict,
            Permission,
            Quota,
        }
        if input.len() > MAX_INPUT_BYTES {
            return Err(Error::Bounds);
        }
        RESPONSE.with(|b| b.borrow_mut().clear());
        // SAFETY: input is an owned immutable domain serialization. JS copies it
        // before reentering run(), which has its own stack Host. No thread-local
        // buffer borrow or reference to this Host crosses the import. The JS
        // aggregate call/deadline guards remain active across both Wasm entries.
        let failed = unsafe { domain(input.as_ptr(), input.len() as u32) };
        if failed != 0 {
            return Err(Error::Storage);
        }
        let response = RESPONSE.with(|b| std::mem::take(&mut *b.borrow_mut()));
        match serde_json::from_slice(&response).map_err(|_| Error::Storage)? {
            DomainResult::Success(reply) => Ok(reply),
            DomainResult::Rejected(Rejection::Conflict) => Err(Error::Conflict),
            DomainResult::Rejected(Rejection::Permission) => Err(Error::Permission),
            DomainResult::Rejected(Rejection::Quota) => Err(Error::Quota),
        }
    }
}

#[no_mangle]
pub extern "C" fn run() -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = wire::run(&mut Host { calls: 0 }, &bytes);
    let (status, output) = match result {
        Ok(reply) => (
            0,
            serde_json::to_vec(&reply).expect("serializable bounded reply"),
        ),
        Err(error) => (
            1,
            serde_json::to_vec(error.code()).expect("static error code"),
        ),
    };
    OUTPUT.with(|b| *b.borrow_mut() = output);
    status
}

/// Private trusted-host entry point; the host authenticates and constructs caller
/// context. Public request bodies cannot choose controller capability.
#[no_mangle]
pub extern "C" fn run_authority() -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = serde_json::from_slice::<crate::authority::Invocation>(&bytes)
        .map_err(|_| Error::Invalid("authority invocation"))
        .and_then(|invocation| crate::authority::run(&mut Host { calls: 0 }, invocation));
    let (status, output) = match result {
        Ok(reply) => (
            0,
            serde_json::to_vec(&reply).expect("bounded authority reply"),
        ),
        Err(error) => (
            1,
            serde_json::to_vec(error.code()).expect("static error code"),
        ),
    };
    OUTPUT.with(|b| *b.borrow_mut() = output);
    status
}

/// Separate real-personal qualification entry. It accepts no synthetic principal
/// or controller selector. Only the trusted host constructs parsed peer parts.
#[no_mangle]
pub extern "C" fn run_personal() -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = serde_json::from_slice::<crate::authority::PersonalInvocation>(&bytes)
        .map_err(|_| crate::personal::PersonalError::Kernel(Error::Invalid("personal invocation")))
        .and_then(|invocation| crate::authority::run_personal(&mut Host { calls: 0 }, invocation));
    personal_output(result)
}

#[no_mangle]
pub extern "C" fn read_personal() -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let result = serde_json::from_slice::<crate::authority::PersonalPointInvocation>(&bytes)
        .map_err(|_| {
            crate::personal::PersonalError::Kernel(Error::Invalid("personal point invocation"))
        })
        .and_then(|invocation| {
            crate::authority::read_personal_point(&mut Host { calls: 0 }, invocation)
        });
    personal_output(result)
}

fn personal_output<T: serde::Serialize>(result: crate::personal::Result<T>) -> u32 {
    let (status, output) = match result {
        Ok(reply) => (
            0,
            serde_json::to_vec(&reply).expect("bounded personal reply"),
        ),
        Err(error) => (
            1,
            serde_json::to_vec(&crate::authority::PersonalErrorPayload::from(&error))
                .expect("bounded typed personal error"),
        ),
    };
    OUTPUT.with(|b| *b.borrow_mut() = output);
    status
}

#[path = "read_token_wasm.rs"]
mod read_token_host;
