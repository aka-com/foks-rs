//! Interactive software-device pairing over the public v0.1.9 KEX relay.

use foks_client_db::{HardStateStore, MutationKind, MutationOperation, MutationState};
use foks_crypto::{
    countersign_software_kex_provision_link, derive_device_public,
    finish_software_kex_provision_link, make_software_kex_provision_link, seal_software_puk_boxes,
    sign_kex_wrapper, DevicePublicMaterial, KexPhrase, KexSecret, PukBoxRandomness,
    SoftwareProvisionInput, SoftwarePukBoxInput, UserMutationBase,
};
use foks_proto::{
    DeviceLabel, DeviceLabelNameAndCommitmentKey, DeviceType, KexActorType, KexCleartext,
    KexDeviceLabelAndName, KexHelloMessage, KexMessage, KexPleaseSign, KexReceiveArgument,
    KexSendArgument, KexWrapperMessage, ProvisionDeviceArgument, Role, SecretSeed,
};
use foks_rpc::{
    encode_kex_receive_request, encode_kex_send_request, encode_provision_device_request,
    encode_registration_select_vhost_request,
};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::{
    now_milliseconds, random_bytes, DeviceCredential, Error, FoksClient, MutationCoordinator,
    PinnedHost, ProtectedMutationStore, ProvisionedSoftwareDevice, Result,
};

/// Total wait a pairing side spends on the KEX relay when the caller states
/// no budget of its own. The two blocking receives share it, so an embedder
/// that cancels pairing after five minutes never cancels a receive this
/// client would still have been waiting on.
pub const DEFAULT_KEX_PAIRING_BUDGET: Duration = Duration::from_secs(5 * 60);
/// Blocking wait asked of the relay per receive RPC.
///
/// A host that honours the request answers within this slice, so raising it
/// cuts a five-minute receive from sixty round trips to ten. Go's relay waits
/// five seconds whatever is requested and then answers TX_RETRY, so against a
/// Go host the slice changes nothing but is still handled by the retry loop
/// below. A receive abandoned by a lost connection outlives that connection by
/// at most one slice, and only on a host that does not notice the socket
/// closing first.
const KEX_SERVER_POLL_SLICE: Duration = Duration::from_secs(30);
const KEX_TRANSPORT_SLACK: Duration = Duration::from_secs(15);

pub struct KexProvisionOffer {
    secret: KexSecret,
    pub role: Role,
}

impl std::fmt::Debug for KexProvisionOffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KexProvisionOffer")
            .field("secret", &"[REDACTED]")
            .field("role", &self.role)
            .finish()
    }
}

impl KexProvisionOffer {
    pub fn generate(role: Role) -> Result<Self> {
        if role == Role::NONE {
            return Err(Error::Kex("provisioned role is empty"));
        }
        Ok(Self {
            secret: KexSecret::generate()?,
            role,
        })
    }

    pub fn phrase(&self) -> KexPhrase {
        self.secret.phrase()
    }

    pub fn into_secret_bytes(self) -> Zeroizing<[u8; foks_proto::KEX_SECRET_BYTES]> {
        self.secret.secret_bytes()
    }

    pub fn from_secret_bytes(
        bytes: [u8; foks_proto::KEX_SECRET_BYTES],
        role: Role,
    ) -> Result<Self> {
        if role == Role::NONE {
            return Err(Error::Kex("provisioned role is empty"));
        }
        Ok(Self {
            secret: KexSecret::from_bytes(bytes)?,
            role,
        })
    }
}

pub struct KexProvisioningReport {
    pub operation_id: Option<[u8; 16]>,
    pub device: DevicePublicMaterial,
}

/// The pairing budget the blocking receives of one pairing half share.
///
/// Each receive asks the relay for whatever is left rather than a fresh full
/// window, so a pairing's total relay wait stays inside the budget its caller
/// granted. An embedder that cancels pairing at its own cap therefore never
/// cancels a receive this client would still have been waiting on, which
/// would report a live pairing as an ambiguous deadline error.
#[derive(Clone, Copy)]
struct KexBudget {
    deadline: Instant,
}

impl KexBudget {
    fn starting_now(budget: Duration) -> Result<Self> {
        Ok(Self {
            deadline: Instant::now()
                .checked_add(budget)
                .ok_or(Error::Kex("KEX pairing budget overflows"))?,
        })
    }

    /// The wait the next receive may ask for. A budget spent down to nothing
    /// is a deadline error: a zero wait is the relay's non-blocking probe,
    /// which answers an unfinished pairing as a bad secret.
    fn remaining(&self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(1) {
            return Err(Error::DeadlineExceeded);
        }
        Ok(remaining)
    }
}

impl FoksClient {
    /// Creates a one-use pairing offer and publishes the KEX `Start` packet.
    /// Persist the returned secret in protected storage before displaying its
    /// phrase. The application should persist the offer before publishing it.
    pub fn publish_kex_provision_offer(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        offer: &KexProvisionOffer,
    ) -> Result<()> {
        self.kex_send(
            host,
            &existing.seed,
            &offer.secret,
            0,
            KexActorType::Provisioner,
            KexMessage::Start,
        )
    }

    /// Completes the provisioner half after the other machine has entered the
    /// offer phrase. The final identity mutation uses the normal durable WAL.
    pub fn finish_kex_provisioning(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        offer: &KexProvisionOffer,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<KexProvisioningReport> {
        self.finish_kex_provisioning_within(
            host,
            existing,
            offer,
            protected_store,
            DEFAULT_KEX_PAIRING_BUDGET,
        )
    }

    /// [`Self::finish_kex_provisioning`] bounded by the caller's own remaining
    /// budget. Both relay receives draw from `budget`, so a caller that
    /// abandons the operation at its cap never abandons a receive still inside
    /// this client's wait.
    pub fn finish_kex_provisioning_within(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        offer: &KexProvisionOffer,
        protected_store: &mut impl ProtectedMutationStore,
        budget: Duration,
    ) -> Result<KexProvisioningReport> {
        let budget = KexBudget::starting_now(budget)?;
        let hello = match self.kex_receive(
            host,
            &existing.seed,
            &offer.secret,
            0,
            KexActorType::Provisioner,
            budget.remaining()?,
        )? {
            KexMessage::Hello(hello) => hello,
            KexMessage::Error(_) => return Err(Error::Kex("peer aborted pairing")),
            _ => return Err(Error::Kex("expected the provisionee hello packet")),
        };
        validate_hello(&hello)?;
        let new_device = DevicePublicMaterial {
            id: hello.entity,
            hepk: hello.hepk,
        };
        let authenticated = self.authenticate_and_pin(host, existing)?;
        let signer = existing.public_material()?;
        if !authenticated
            .verified
            .devices()
            .iter()
            .any(|device| device.id == signer.id && device.role == Role::OWNER)
        {
            return Err(Error::Kex(
                "device provisioning requires an enrolled owner signer",
            ));
        }
        let enrolled = authenticated
            .verified
            .devices()
            .iter()
            .find(|device| device.id == new_device.id);
        let prior = HardStateStore::open(&host.database_path)?.latest_mutation_for_binding(
            host.host_id().as_bytes(),
            MutationKind::DeviceProvision,
            authenticated.verified.uid().as_bytes(),
            new_device.id.as_bytes(),
        )?;
        if let Some(device) = enrolled {
            if device.role != offer.role {
                return Err(Error::OperationBinding(
                    "paired device is enrolled with another role",
                ));
            }
            let operation_id = self.reconcile_enrolled_kex_mutation(
                host,
                existing,
                prior.as_ref(),
                protected_store,
            )?;
            self.kex_send(
                host,
                &existing.seed,
                &offer.secret,
                2,
                KexActorType::Provisioner,
                KexMessage::Done,
            )?;
            return Ok(KexProvisioningReport {
                operation_id,
                device: new_device,
            });
        }
        if let Some(operation) = prior
            .as_ref()
            .filter(|operation| !matches!(operation.state, MutationState::Rejected))
        {
            let operation = self.bound_user_mutation(
                host,
                operation.operation_id,
                MutationKind::DeviceProvision,
                &existing.uid,
                protected_store,
            )?;
            if operation.subject_id != new_device.id.as_bytes() {
                return Err(Error::OperationBinding(
                    "paired device mutation subject changed",
                ));
            }
            if matches!(
                operation.state,
                MutationState::RemoteVerified | MutationState::Finalized
            ) {
                return Err(Error::OperationBinding(
                    "verified paired device is absent from the authenticated chain",
                ));
            }
            let post_error =
                self.resume_user_mutation_submission(host, existing, &operation, protected_store)?;
            match self.wait_for_user_transition(host, existing, |user| {
                user.devices()
                    .iter()
                    .any(|device| device.id == new_device.id && device.role == offer.role)
            }) {
                Ok(_) => {}
                Err(_) if post_error.is_some() => {
                    return Err(post_error.expect("checked above"));
                }
                Err(error) => return Err(error),
            }
            MutationCoordinator::new(&host.database_path, protected_store)
                .remote_verified(&operation.operation_id)?;
            self.kex_send(
                host,
                &existing.seed,
                &offer.secret,
                2,
                KexActorType::Provisioner,
                KexMessage::Done,
            )?;
            return Ok(KexProvisioningReport {
                operation_id: Some(operation.operation_id),
                device: new_device,
            });
        }
        let public_puk = authenticated
            .verified
            .shared_key(offer.role)
            .ok_or(Error::Kex("the provisioned role has no PUK"))?;
        let private_puk = self
            .load_puks_for_role(host, existing, &authenticated.verified, offer.role)?
            .into_iter()
            .find(|key| key.role == offer.role && key.generation == public_puk.generation)
            .ok_or(Error::Kex("the current role PUK is unavailable"))?;
        let next_tree_location = random_bytes()?;
        let commitment_key = random_bytes()?;
        let material = make_software_kex_provision_link(
            &SoftwareProvisionInput {
                base: UserMutationBase {
                    uid: authenticated.verified.uid(),
                    host: authenticated.verified.host(),
                    seqno: authenticated
                        .verified
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(Error::Kex("user sequence overflow"))?,
                    previous: authenticated.verified.chain_tail_hash(),
                    root: &authenticated.verified.tree_root(),
                    time: now_milliseconds()?,
                    next_tree_location,
                },
                role: offer.role,
                device_label: &hello.device_name.label,
                device_name_commitment_key: commitment_key,
            },
            &existing.seed,
            &new_device,
        )?;
        let puk_boxes = seal_software_puk_boxes(
            host.host_id(),
            &existing.seed,
            random_bytes()?,
            &[SoftwarePukBoxInput {
                seed: &private_puk.seed,
                generation: private_puk.generation,
                role: private_puk.role,
                receiver: &new_device,
            }],
            &[PukBoxRandomness {
                kem_message: random_bytes()?,
                nonce: random_bytes()?,
            }],
        )?;
        let mut self_token: [u8; foks_proto::KEX_PERMISSION_TOKEN_BYTES] = random_bytes()?;
        self_token[0] = 54;
        let ppe = if offer.role == Role::OWNER {
            self.kex_passphrase_package(host, existing, &authenticated, &private_puk.seed)?
        } else {
            None
        };
        self.kex_send(
            host,
            &existing.seed,
            &offer.secret,
            1,
            KexActorType::Provisioner,
            KexMessage::PleaseSign(KexPleaseSign {
                link: material.link.clone(),
                ppe,
                self_token,
            }),
        )?;
        let signed = match self.kex_receive(
            host,
            &existing.seed,
            &offer.secret,
            1,
            KexActorType::Provisioner,
            budget.remaining()?,
        )? {
            KexMessage::OkSigned(signature) => material.link.with_appended_signature(signature)?,
            KexMessage::Error(_) => return Err(Error::Kex("peer aborted pairing")),
            _ => return Err(Error::Kex("expected the provisionee signature packet")),
        };
        let link = finish_software_kex_provision_link(&signed, &existing.seed, &new_device)?;
        let device_name = DeviceLabelNameAndCommitmentKey {
            label: hello.device_name.label,
            normalization_version: hello.device_name.normalization_version,
            display_name: hello.device_name.display_name,
            commitment_key,
        };
        let encoded = Zeroizing::new(encode_provision_device_request(&ProvisionDeviceArgument {
            link: &link,
            puk_boxes: &puk_boxes,
            device_name: &device_name,
            next_tree_location,
            self_token,
            hepks: std::slice::from_ref(&new_device.hepk),
            subkey_box: None,
            yubi_pq_hint: None,
        })?);
        let operation_id = self.prepare_user_mutation(
            host,
            MutationKind::DeviceProvision,
            authenticated.verified.uid(),
            &new_device.id,
            authenticated.verified.chain_seqno() + 1,
            &encoded,
            protected_store,
        )?;
        if let Some(error) =
            self.submit_user_mutation(host, existing, operation_id, &encoded, protected_store)?
        {
            return Err(error);
        }
        self.wait_for_user_transition(host, existing, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == new_device.id && device.role == offer.role)
        })?;
        self.kex_send(
            host,
            &existing.seed,
            &offer.secret,
            2,
            KexActorType::Provisioner,
            KexMessage::Done,
        )?;
        MutationCoordinator::new(&host.database_path, protected_store)
            .remote_verified(&operation_id)?;
        Ok(KexProvisioningReport {
            operation_id: Some(operation_id),
            device: new_device,
        })
    }

    fn reconcile_enrolled_kex_mutation(
        &self,
        host: &PinnedHost,
        existing: &DeviceCredential,
        operation: Option<&MutationOperation>,
        protected_store: &mut impl ProtectedMutationStore,
    ) -> Result<Option<[u8; 16]>> {
        let Some(operation) = operation else {
            return Ok(None);
        };
        match operation.state {
            MutationState::Submitting | MutationState::SubmissionUnknown => {
                let operation = self.bound_user_mutation(
                    host,
                    operation.operation_id,
                    MutationKind::DeviceProvision,
                    &existing.uid,
                    protected_store,
                )?;
                MutationCoordinator::new(&host.database_path, protected_store)
                    .remote_verified(&operation.operation_id)?;
                Ok(Some(operation.operation_id))
            }
            MutationState::RemoteVerified | MutationState::Finalized => {
                Ok(Some(operation.operation_id))
            }
            MutationState::Prepared => Err(Error::OperationBinding(
                "prepared pairing mutation cannot have changed remote state",
            )),
            MutationState::Rejected => Ok(None),
        }
    }

    /// Runs the provisionee half on a fresh machine using the displayed HESP
    /// phrase and returns a fully authenticated local device credential.
    pub fn accept_kex_provisioning(
        &self,
        host: &PinnedHost,
        phrase: &str,
        device_name: &str,
        serial: u64,
        device_seed: SecretSeed,
    ) -> Result<ProvisionedSoftwareDevice> {
        self.accept_kex_provisioning_for_user(host, phrase, device_name, serial, device_seed, None)
    }

    pub fn accept_kex_provisioning_for_user(
        &self,
        host: &PinnedHost,
        phrase: &str,
        device_name: &str,
        serial: u64,
        device_seed: SecretSeed,
        expected_user: Option<&[u8; 33]>,
    ) -> Result<ProvisionedSoftwareDevice> {
        self.accept_kex_provisioning_for_user_within(
            host,
            phrase,
            device_name,
            serial,
            device_seed,
            expected_user,
            DEFAULT_KEX_PAIRING_BUDGET,
        )
    }

    /// [`Self::accept_kex_provisioning_for_user`] bounded by the caller's own
    /// remaining budget; see [`Self::finish_kex_provisioning_within`].
    #[allow(clippy::too_many_arguments)]
    pub fn accept_kex_provisioning_for_user_within(
        &self,
        host: &PinnedHost,
        phrase: &str,
        device_name: &str,
        serial: u64,
        device_seed: SecretSeed,
        expected_user: Option<&[u8; 33]>,
        budget: Duration,
    ) -> Result<ProvisionedSoftwareDevice> {
        let budget = KexBudget::starting_now(budget)?;
        if serial == 0 {
            return Err(Error::Kex("device serial is zero"));
        }
        let secret = KexSecret::from_phrase(phrase)?;
        let display_name = crate::prepare_device_name(device_name)?;
        let normalized_name = crate::normalize_device_name(display_name.as_bytes())
            .ok_or(Error::Kex("device name is invalid"))?;
        let public = derive_device_public(&device_seed)?;
        // The Start packet is already queued when the phrase is typed, so this
        // probe never blocks and never draws on the pairing budget.
        match self.kex_receive(
            host,
            &device_seed,
            &secret,
            0,
            KexActorType::Provisionee,
            Duration::ZERO,
        )? {
            KexMessage::Start => {}
            _ => return Err(Error::Kex("pairing phrase has no Start packet")),
        }
        self.kex_send(
            host,
            &device_seed,
            &secret,
            0,
            KexActorType::Provisionee,
            KexMessage::Hello(KexHelloMessage {
                entity: public.id.clone(),
                hepk: public.hepk.clone(),
                device_name: KexDeviceLabelAndName {
                    label: DeviceLabel {
                        device_type: DeviceType::Computer,
                        normalized_name,
                        serial,
                    },
                    normalization_version: 0,
                    display_name: display_name.into_bytes(),
                },
            }),
        )?;
        let request = match self.kex_receive(
            host,
            &device_seed,
            &secret,
            1,
            KexActorType::Provisionee,
            budget.remaining()?,
        )? {
            KexMessage::PleaseSign(request) => request,
            KexMessage::Error(_) => return Err(Error::Kex("peer aborted pairing")),
            _ => return Err(Error::Kex("expected a provision link")),
        };
        let change = request.link.decode_group_change()?;
        if change.host != *host.host_id() || change.changes.len() != 1 {
            return Err(Error::Kex(
                "provision link targets the wrong host or device set",
            ));
        }
        if expected_user.is_some_and(|user| change.uid.as_bytes() != user) {
            return Err(Error::Kex("provision link targets the wrong user"));
        }
        let signed = countersign_software_kex_provision_link(&request.link, &device_seed)?;
        let signature = signed
            .signatures()
            .last()
            .cloned()
            .ok_or(Error::Kex("provisionee signature is absent"))?;
        self.kex_send(
            host,
            &device_seed,
            &secret,
            1,
            KexActorType::Provisionee,
            KexMessage::OkSigned(signature),
        )?;
        match self.kex_receive(
            host,
            &device_seed,
            &secret,
            2,
            KexActorType::Provisionee,
            budget.remaining()?,
        )? {
            KexMessage::Done => {}
            KexMessage::Error(_) => return Err(Error::Kex("peer rejected pairing")),
            _ => return Err(Error::Kex("expected the final Done packet")),
        }
        let uid = change.uid;
        let role = change.changes[0].role;
        let certificate_chain = self.fetch_device_certificate_chain(host, &uid, &device_seed)?;
        let credential = DeviceCredential {
            key_kind: crate::SoftwareKeyKind::Device,
            uid,
            seed: device_seed,
            certificate_chain,
        };
        self.probe_key_exists(
            host,
            &credential.uid,
            &public.id,
            &foks_proto::PermissionToken::new(request.self_token),
        )?;
        let authenticated = self.wait_for_user_transition(host, &credential, |user| {
            user.devices()
                .iter()
                .any(|device| device.id == public.id && device.role == role)
        })?;
        Ok(ProvisionedSoftwareDevice {
            operation_id: None,
            credential,
            authenticated,
        })
    }

    fn kex_send(
        &self,
        host: &PinnedHost,
        seed: &SecretSeed,
        secret: &KexSecret,
        sequence: u64,
        actor: KexActorType,
        message: KexMessage,
    ) -> Result<()> {
        let keys = secret.keys()?;
        let sender = derive_device_public(seed)?.id;
        let cleartext = KexCleartext {
            session_id: keys.session_id,
            sender: sender.clone(),
            sequence,
            message,
        };
        let wrapper = KexWrapperMessage {
            session_id: keys.session_id,
            sender,
            sequence,
            payload: keys.seal(&cleartext, random_bytes()?)?,
        };
        let request = encode_kex_send_request(&KexSendArgument {
            signature: sign_kex_wrapper(seed, &wrapper)?,
            message: wrapper,
            actor,
        })?;
        self.call_void_after_vhost_selection(
            host,
            &host.registration,
            &encode_registration_select_vhost_request(host.host_id())?,
            &request,
        )
    }

    fn kex_receive(
        &self,
        host: &PinnedHost,
        seed: &SecretSeed,
        secret: &KexSecret,
        sequence: u64,
        actor: KexActorType,
        requested_wait: Duration,
    ) -> Result<KexMessage> {
        let keys = secret.keys()?;
        let receiver = derive_device_public(seed)?.id;
        // The relay answers a blocking receive in bounded slices and TX_RETRY
        // when a slice expires, so a lost connection leaves at most one slice
        // occupying server resources instead of the whole pairing wait. Retry
        // those slices until the caller's remaining pairing budget is spent.
        let deadline = Instant::now()
            .checked_add(requested_wait)
            .ok_or(Error::Kex("KEX poll deadline overflows"))?;
        let response = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let poll_slice = if requested_wait.is_zero() {
                Duration::ZERO
            } else {
                remaining.min(KEX_SERVER_POLL_SLICE)
            };
            let mut polling_client = self.clone();
            polling_client.set_timeout(kex_poll_transport_timeout(poll_slice));
            let request = encode_kex_receive_request(&KexReceiveArgument {
                session_id: keys.session_id,
                receiver: receiver.clone(),
                sequence,
                poll_wait_milliseconds: poll_slice.as_millis() as u64,
                actor,
            })?;
            match polling_client.call_after_vhost_selection(
                host,
                &host.registration,
                &encode_registration_select_vhost_request(host.host_id())?,
                &request,
            ) {
                Ok(response) => break response,
                Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1014, .. }))
                    if !requested_wait.is_zero() && Instant::now() < deadline =>
                {
                    continue;
                }
                Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1014, .. }))
                    if !requested_wait.is_zero() =>
                {
                    return Err(Error::DeadlineExceeded);
                }
                Err(error) => return Err(error),
            }
        };
        let wrapper = KexWrapperMessage::decode(&response)?;
        if wrapper.session_id != keys.session_id
            || wrapper.sequence != sequence
            || wrapper.sender == receiver
        {
            return Err(Error::Kex("relay returned a rebound or reflected packet"));
        }
        // The relay verifies each wrapper signature before insertion. The
        // signed wrapper does not carry that signature back on receive, so the
        // client authenticates the sender again through the encrypted packet's
        // exact sender/session/sequence bindings and, for Hello/OkSigned, the
        // identity-chain countersignature before mutation acceptance.
        Ok(keys.open(&wrapper)?.message)
    }
}

fn kex_poll_transport_timeout(poll_wait: Duration) -> Duration {
    poll_wait.saturating_add(KEX_TRANSPORT_SLACK)
}

fn validate_hello(hello: &KexHelloMessage) -> Result<()> {
    hello
        .entity
        .clone()
        .require_type(foks_proto::ENTITY_DEVICE)?;
    if hello.device_name.label.device_type != DeviceType::Computer
        || hello.device_name.label.serial == 0
        || crate::normalize_device_name(&hello.device_name.display_name).as_deref()
            != Some(hello.device_name.label.normalized_name.as_slice())
    {
        return Err(Error::Kex("provisionee device label is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kex_poll_deadline_outlives_the_advertised_wait() {
        assert_eq!(
            kex_poll_transport_timeout(KEX_SERVER_POLL_SLICE),
            KEX_SERVER_POLL_SLICE + KEX_TRANSPORT_SLACK
        );
        assert!(kex_poll_transport_timeout(KEX_SERVER_POLL_SLICE) > Duration::from_secs(15));
    }

    #[test]
    fn kex_poll_slice_bounds_the_round_trips_of_a_full_budget() {
        assert_eq!(KEX_SERVER_POLL_SLICE, Duration::from_secs(30));
        let slices = DEFAULT_KEX_PAIRING_BUDGET.as_secs() / KEX_SERVER_POLL_SLICE.as_secs();
        assert_eq!(slices, 10);
    }

    #[test]
    fn kex_budget_is_shared_by_successive_receives() {
        let budget = KexBudget::starting_now(DEFAULT_KEX_PAIRING_BUDGET).unwrap();
        let first = budget.remaining().unwrap();
        let second = budget.remaining().unwrap();
        assert!(first <= DEFAULT_KEX_PAIRING_BUDGET);
        assert!(second <= first);
        // Two receives out of one budget can never exceed it, unlike two
        // independent full poll windows.
        assert!(first.saturating_sub(second) < DEFAULT_KEX_PAIRING_BUDGET);
    }

    #[test]
    fn spent_kex_budget_reports_a_deadline_rather_than_a_probe() {
        let budget = KexBudget::starting_now(Duration::ZERO).unwrap();
        assert!(matches!(budget.remaining(), Err(Error::DeadlineExceeded)));
    }

    #[test]
    fn default_kex_pairing_budget_is_five_minutes_in_total() {
        assert_eq!(DEFAULT_KEX_PAIRING_BUDGET, Duration::from_secs(5 * 60));
    }
}
