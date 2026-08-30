use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use foks_proto::{
    KexReceiveArgument, KexSendArgument, KexWrapperMessage, ENTITY_DEVICE, ENTITY_YUBI,
};
use foks_rpc::RpcStatus;

const MAX_MESSAGES: usize = 4096;
const MAX_RELAY_BYTES: usize = 32 * 1024 * 1024;
const MESSAGE_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);
pub(super) const MAX_BLOCKING_WAIT: Duration = Duration::from_secs(60 * 60);
pub(super) const MAXIMUM_WAITERS: usize = 16;

struct Queued {
    inserted: Instant,
    encoded_bytes: usize,
    message: KexWrapperMessage,
}

#[derive(Default)]
struct State {
    messages: VecDeque<Queued>,
    encoded_bytes: usize,
}

#[derive(Default)]
pub(super) struct Relay {
    state: Mutex<State>,
    changed: Condvar,
}

impl Relay {
    pub(super) fn send(&self, encoded: &[u8]) -> Result<(), RpcStatus> {
        let argument = KexSendArgument::decode(encoded).map_err(bad_arguments)?;
        require_device(&argument.message.sender)?;
        foks_crypto::verify_kex_wrapper(&argument.message, &argument.signature)
            .map_err(bad_arguments)?;
        let mut state = self.state.lock().map_err(|_| RpcStatus::TransactionRetry)?;
        cleanup(&mut state, Instant::now());
        if state.messages.iter().any(|queued| {
            queued.message.session_id == argument.message.session_id
                && queued.message.sequence == argument.message.sequence
                && queued.message.sender == argument.message.sender
        }) {
            // The authenticated `(session, sender, seqno)` slot is immutable
            // to receivers. Treat retransmission as success so a lost send
            // response does not wedge the pairing.
            return Ok(());
        }
        let encoded_bytes = argument.message.encoded().map_err(bad_arguments)?.len();
        while state.messages.len() == MAX_MESSAGES
            || state
                .encoded_bytes
                .checked_add(encoded_bytes)
                .is_none_or(|total| total > MAX_RELAY_BYTES)
        {
            let Some(evicted) = state.messages.pop_front() else {
                return Err(RpcStatus::RateLimited);
            };
            state.encoded_bytes = state.encoded_bytes.saturating_sub(evicted.encoded_bytes);
        }
        state.encoded_bytes += encoded_bytes;
        state.messages.push_back(Queued {
            inserted: Instant::now(),
            encoded_bytes,
            message: argument.message,
        });
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn receive(&self, encoded: &[u8]) -> Result<KexWrapperMessage, RpcStatus> {
        let argument = KexReceiveArgument::decode(encoded).map_err(bad_arguments)?;
        require_device(&argument.receiver)?;
        let requested_wait = Duration::from_millis(argument.poll_wait_milliseconds);
        let deadline = Instant::now()
            .checked_add(requested_wait.min(MAX_BLOCKING_WAIT))
            .ok_or_else(|| bad_arguments("KEX poll deadline overflows"))?;
        let mut state = self.state.lock().map_err(|_| RpcStatus::TransactionRetry)?;
        loop {
            let now = Instant::now();
            cleanup(&mut state, now);
            if let Some(message) = state
                .messages
                .iter()
                .find(|queued| {
                    queued.message.session_id == argument.session_id
                        && queued.message.sequence == argument.sequence
                        && queued.message.sender != argument.receiver
                })
                .map(|queued| queued.message.clone())
            {
                return Ok(message);
            }
            if requested_wait.is_zero() {
                return Err(RpcStatus::KexBadSecret);
            }
            let remaining = deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                return Err(RpcStatus::TransactionRetry);
            }
            let (next, _) = self
                .changed
                .wait_timeout(state, remaining.min(Duration::from_secs(1)))
                .map_err(|_| RpcStatus::TransactionRetry)?;
            state = next;
        }
    }
}

fn cleanup(state: &mut State, now: Instant) {
    let mut retained_bytes = 0usize;
    state.messages.retain(|queued| {
        let retained = now
            .checked_duration_since(queued.inserted)
            .is_some_and(|age| age <= MESSAGE_LIFETIME);
        if retained {
            retained_bytes = retained_bytes.saturating_add(queued.encoded_bytes);
        }
        retained
    });
    state.encoded_bytes = retained_bytes;
}

fn require_device(entity: &foks_proto::EntityId) -> Result<(), RpcStatus> {
    if matches!(entity.entity_type(), ENTITY_DEVICE | ENTITY_YUBI) {
        Ok(())
    } else {
        Err(bad_arguments("KEX sender/receiver is not a device"))
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use foks_proto::{KexActorType, SecretBox, SecretSeed};

    use super::*;

    fn packet() -> (KexSendArgument, foks_proto::EntityId) {
        let sender_seed = SecretSeed::new([0x31; 32]);
        let receiver_seed = SecretSeed::new([0x41; 32]);
        let sender = foks_crypto::derive_device_public(&sender_seed).unwrap().id;
        let receiver = foks_crypto::derive_device_public(&receiver_seed)
            .unwrap()
            .id;
        let message = KexWrapperMessage {
            session_id: [0x51; 32],
            sender,
            sequence: 2,
            payload: SecretBox {
                nonce: [0x61; 16],
                ciphertext: vec![0x71; 32],
            },
        };
        let signature = foks_crypto::sign_kex_wrapper(&sender_seed, &message).unwrap();
        (
            KexSendArgument {
                message,
                signature,
                actor: KexActorType::Provisioner,
            },
            receiver,
        )
    }

    #[test]
    fn long_poll_wakes_without_consuming_the_packet() {
        let relay = Arc::new(Relay::default());
        let (send, receiver) = packet();
        let receive = KexReceiveArgument {
            session_id: send.message.session_id,
            receiver,
            sequence: send.message.sequence,
            poll_wait_milliseconds: 2_000,
            actor: KexActorType::Provisionee,
        };
        let waiting = {
            let relay = Arc::clone(&relay);
            let encoded = receive.encoded().unwrap();
            std::thread::spawn(move || relay.receive(&encoded).unwrap())
        };
        std::thread::sleep(Duration::from_millis(25));
        relay.send(&send.encoded().unwrap()).unwrap();
        assert_eq!(waiting.join().unwrap(), send.message);
        assert_eq!(
            relay.receive(&receive.encoded().unwrap()).unwrap(),
            send.message
        );
    }

    #[test]
    fn relay_rejects_invalid_wrapper_signatures() {
        let relay = Relay::default();
        let (mut send, _) = packet();
        send.message.sequence += 1;
        assert!(matches!(
            relay.send(&send.encoded().unwrap()),
            Err(RpcStatus::BadArguments(_))
        ));
    }
}
