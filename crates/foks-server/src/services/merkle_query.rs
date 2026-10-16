//! Merkle query routes and the current-root answers of the probe service.

use foks_proto::{
    EntityId, HistoricalMerkleRoots, MerkleExistsResponse, MerkleLookupResponse,
    MerkleMultiLookupResponse, MerkleRoot, ProbeResponse, SignedBlob, TreeRoot,
};
use foks_rpc::arguments::{
    decode_merkle_check_key, decode_merkle_lookup, decode_merkle_multi_lookup,
    decode_merkle_optional_host,
};
use foks_rpc::RpcStatus;
use foks_snowpack::{decode, Value};

use super::user::bad_arguments;
use crate::merkle::decode_stored_root;

/// The server state that Merkle queries read. Without a read database the
/// server answers from its bootstrap probe response.
pub(crate) struct MerkleQueries<'a> {
    pub(crate) host_id: &'a [u8],
    pub(crate) read_database: Option<&'a crate::read_pool::ReadPool>,
    pub(crate) metrics: &'a crate::ServerMetrics,
    /// The bootstrap SignedBlob Merkle root.
    pub(crate) current_root: &'a [u8],
    pub(crate) probe_response: &'a [u8],
}

impl MerkleQueries<'_> {
    fn read_database(&self) -> Result<crate::read_pool::ReadLease, RpcStatus> {
        let pool = self.read_database.ok_or(RpcStatus::Unsupported)?;
        pool.lease(self.metrics)
    }

    /// getCurrentRoot @2 returns the bare unsigned MerkleRoot, matching the
    /// v0.1.9 wire contract. The authenticated advance path must instead use
    /// getCurrentRootSigned @5 (below) so it can verify the signature.
    pub(crate) fn current_root(&self) -> Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            let signed =
                SignedBlob::decode(self.current_root).map_err(|_| RpcStatus::TransactionRetry)?;
            return Ok(signed.inner);
        }
        let root = self.validated_current_root()?;
        Ok(root.exact_root)
    }

    /// getCurrentRootSigned @5 returns the delegated Merkle-signer SignedBlob so
    /// the client can authenticate the current root before accepting it.
    pub(crate) fn current_root_signed(&self) -> Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            return Ok(self.current_root.to_vec());
        }
        let root = self.validated_current_root()?;
        Ok(root.exact_signed_root)
    }

    fn validated_current_root(&self) -> Result<foks_server_db::RootSnapshot, RpcStatus> {
        let database = self.read_database()?;
        let root = database
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        validated_root(&root)?;
        let signed =
            SignedBlob::decode(&root.exact_signed_root).map_err(|_| RpcStatus::TransactionRetry)?;
        if signed.inner != root.exact_root {
            return Err(RpcStatus::TransactionRetry);
        }
        Ok(root)
    }

    pub(crate) fn current_probe_response(&self) -> Result<Vec<u8>, RpcStatus> {
        if self.read_database.is_none() {
            return Ok(self.probe_response.to_vec());
        }
        let database = self.read_database()?;
        let root = database
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        validated_root(&root)?;
        let signed =
            SignedBlob::decode(&root.exact_signed_root).map_err(|_| RpcStatus::TransactionRetry)?;
        if signed.inner != root.exact_root {
            return Err(RpcStatus::TransactionRetry);
        }
        let mut probe =
            ProbeResponse::decode(self.probe_response).map_err(|_| RpcStatus::TransactionRetry)?;
        probe.merkle_root = signed;
        probe.encoded().map_err(|_| RpcStatus::TransactionRetry)
    }

    pub(crate) fn historical_roots(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let (full_epochs, hash_epochs) = decode_historical_roots_argument(argument, self.host_id)?;
        if self.read_database.is_none() {
            return Err(RpcStatus::Unsupported);
        }
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let full = snapshot
            .roots_at(&full_epochs)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        let hashes = snapshot
            .roots_at(&hash_epochs)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
        for root in &hashes {
            validated_root(root)?;
        }
        HistoricalMerkleRoots {
            roots: full
                .iter()
                .map(validated_root)
                .collect::<Result<Vec<_>, _>>()?,
            hashes: hashes.into_iter().map(|root| root.root_hash).collect(),
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    pub(crate) fn merkle_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_lookup(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let root = select_merkle_lookup_root(&snapshot, argument.signed, argument.root)?;
        let path = foks_merkle_store::proof(&snapshot.node_reader(), root.root_node, argument.key)
            .map_err(map_merkle_proof_error)?;
        MerkleLookupResponse {
            root: validated_root(&root)?,
            path,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    pub(crate) fn merkle_multi_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_multi_lookup(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let root = select_merkle_lookup_root(&snapshot, argument.signed, argument.root)?;
        let reader = snapshot.node_reader();
        let paths = argument
            .keys
            .into_iter()
            .map(|key| {
                foks_merkle_store::proof(&reader, root.root_node, key)
                    .map_err(map_merkle_proof_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        MerkleMultiLookupResponse {
            root: validated_root(&root)?,
            paths,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    pub(crate) fn current_root_hash(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let host = decode_merkle_optional_host(argument).map_err(bad_arguments)?;
        self.validate_optional_host(host.as_ref())?;
        let root = self
            .read_database()?
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::MerkleNoRoot)?;
        validated_root(&root)?;
        TreeRoot {
            epoch: root.epoch,
            hash: root.root_hash,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    pub(crate) fn merkle_check_key_exists(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let argument = decode_merkle_check_key(argument).map_err(bad_arguments)?;
        self.validate_optional_host(argument.host.as_ref())?;
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let epoch = snapshot
            .merkle_leaf_epoch(&argument.key)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::MerkleLeafNotFound)?;
        MerkleExistsResponse {
            epoch,
            // Root publication and signing are one atomic standalone commit.
            signed: true,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn validate_optional_host(&self, host: Option<&EntityId>) -> Result<(), RpcStatus> {
        if host.is_some_and(|host| host.as_bytes() != self.host_id) {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        Ok(())
    }
}

fn select_merkle_lookup_root(
    snapshot: &foks_server_db::ReadSnapshot<'_>,
    signed: bool,
    epoch: Option<u64>,
) -> Result<foks_server_db::RootSnapshot, RpcStatus> {
    if epoch.is_some_and(|epoch| i64::try_from(epoch).is_err()) {
        return Err(RpcStatus::MerkleNoRoot);
    }
    let root = match epoch {
        Some(epoch) => snapshot
            .roots_at(&[epoch])
            .map_err(|_| RpcStatus::TransactionRetry)?
            .and_then(|mut roots| roots.pop()),
        None => snapshot
            .current_root()
            .map_err(|_| RpcStatus::TransactionRetry)?,
    }
    .ok_or(RpcStatus::MerkleNoRoot)?;
    validated_root(&root)?;
    if signed {
        let signed =
            SignedBlob::decode(&root.exact_signed_root).map_err(|_| RpcStatus::TransactionRetry)?;
        if signed.inner != root.exact_root {
            return Err(RpcStatus::TransactionRetry);
        }
    }
    Ok(root)
}

fn map_merkle_proof_error(error: foks_merkle_store::Error) -> RpcStatus {
    match error {
        // Storage access can be transient. Every other proof failure means
        // that the selected authenticated root cannot be traversed as a
        // valid content-addressed tree and is the Go MERKLE_VERIFY_ERROR.
        error @ foks_merkle_store::Error::Storage(_) => {
            crate::status::internal(error, "merkle_proof")
        }
        _ => RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned()),
    }
}

pub(crate) fn validate_host_argument_against(
    argument: &[u8],
    expected_host: &[u8],
    allow_omitted: bool,
) -> Result<(), RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("host argument is not a struct"));
    };
    let [host] = fields.as_slice() else {
        return Err(bad_arguments("host argument has the wrong shape"));
    };
    if allow_omitted && matches!(host, Value::Null) {
        return Ok(());
    }
    validate_host_value_against(host, expected_host)
}

fn validate_host_value_against(host: &Value, expected_host: &[u8]) -> Result<(), RpcStatus> {
    let Value::Binary(host) = host else {
        return Err(bad_arguments("host argument is not binary"));
    };
    if host.len() != 33 || host.first() != Some(&foks_proto::ENTITY_HOST) {
        return Err(bad_arguments("host argument is not a HostID"));
    }
    if host.as_slice() != expected_host {
        return Err(RpcStatus::NotFound("host not found".to_owned()));
    }
    Ok(())
}

fn validate_optional_host_value_against(
    host: &Value,
    expected_host: &[u8],
) -> Result<(), RpcStatus> {
    if matches!(host, Value::Null) {
        return Ok(());
    }
    validate_host_value_against(host, expected_host)
}

fn decode_historical_roots_argument(
    argument: &[u8],
    expected_host: &[u8],
) -> Result<(Vec<u64>, Vec<u64>), RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("historical roots argument is not a struct"));
    };
    let [host, full_epochs, hash_epochs] = fields.as_slice() else {
        return Err(bad_arguments(
            "historical roots argument has the wrong shape",
        ));
    };
    validate_optional_host_value_against(host, expected_host)?;
    Ok((decode_epochs(full_epochs)?, decode_epochs(hash_epochs)?))
}

pub(crate) fn validated_root(root: &foks_server_db::RootSnapshot) -> Result<MerkleRoot, RpcStatus> {
    decode_stored_root(root).map_err(|_| RpcStatus::TransactionRetry)
}

fn decode_epochs(value: &Value) -> Result<Vec<u64>, RpcStatus> {
    let values = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(values) if values.len() <= 64 => values,
        Value::Array(_) => return Err(bad_arguments("too many historical root epochs")),
        _ => return Err(bad_arguments("historical root epochs are not a list")),
    };
    values
        .iter()
        .map(|value| match value {
            Value::Unsigned(epoch) => Ok(*epoch),
            _ => Err(bad_arguments("historical root epoch is not unsigned")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_integrity_failures_are_not_laundered_as_retries() {
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::HashMismatch),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::MissingNode([0x41; 32])),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::EmptyTree),
            RpcStatus::MerkleVerify("Merkle proof verification failed".to_owned())
        );
        assert_eq!(
            map_merkle_proof_error(foks_merkle_store::Error::Storage("busy".to_owned())),
            RpcStatus::TransactionRetry
        );
    }

    #[test]
    fn merkle_query_hosts_may_be_omitted_but_vhost_selection_remains_strict() {
        let mut host = vec![0x41; 33];
        host[0] = foks_proto::ENTITY_HOST;
        let omitted = foks_snowpack::encode(&Value::Array(vec![Value::Null])).unwrap();
        let explicit =
            foks_snowpack::encode(&Value::Array(vec![Value::Binary(host.clone())])).unwrap();
        let mut other_host = host.clone();
        other_host[1] ^= 1;
        let mismatched =
            foks_snowpack::encode(&Value::Array(vec![Value::Binary(other_host.clone())])).unwrap();

        validate_host_argument_against(&omitted, &host, true).unwrap();
        validate_host_argument_against(&explicit, &host, true).unwrap();
        validate_host_argument_against(&explicit, &host, false).unwrap();
        validate_optional_host_value_against(&Value::Null, &host).unwrap();
        validate_optional_host_value_against(&Value::Binary(host.clone()), &host).unwrap();
        let omitted_historical = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-roots-omitted-host-request.frame",
            ),
        )
        .unwrap();
        let call = foks_rpc::read_call(
            &mut std::io::Cursor::new(omitted_historical),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )
        .unwrap();
        assert_eq!(
            decode_historical_roots_argument(call.argument(), &host).unwrap(),
            (vec![996], vec![997, 996, 994, 992])
        );
        assert!(matches!(
            validate_host_argument_against(&mismatched, &host, true),
            Err(RpcStatus::NotFound(_))
        ));
        assert!(matches!(
            validate_optional_host_value_against(&Value::Binary(other_host), &host),
            Err(RpcStatus::NotFound(_))
        ));
        assert!(matches!(
            validate_host_argument_against(&omitted, &host, false),
            Err(RpcStatus::BadArguments(_))
        ));
    }
}
