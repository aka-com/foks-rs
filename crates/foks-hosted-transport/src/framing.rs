//! Bounded FOKS framing and local write/flush completion, without a reply owner.
use crate::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub async fn read_frame_ingress<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum: usize,
    budget: std::time::Duration,
) -> foks_rpc::Result<(usize, crate::deadline::Deadline)> {
    // Idle time is not request work. Once the first byte arrives, the rest of
    // the frame and its dispatch consume this deadline; the FOKS server gives
    // the response write its own allowance.
    let marker = read_byte_async(reader).await?;
    let deadline = crate::deadline::Deadline::after(budget)
        .map_err(|error| foks_rpc::Error::Io(std::io::Error::other(error)))?;
    let length = tokio::time::timeout_at(
        deadline.instant().into(),
        read_frame_length_after_marker(reader, maximum, marker),
    )
    .await
    .map_err(|_| foks_rpc::Error::Io(timeout_error("frame deadline expired")))??;
    Ok((length, deadline))
}

async fn read_frame_length_after_marker<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum: usize,
    marker: u8,
) -> foks_rpc::Result<usize> {
    let length = match marker {
        0x00..=0x7f => usize::from(marker),
        0xcc => {
            let value = usize::from(read_byte_async(reader).await?);
            if value <= 0x7f {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        0xcd => {
            let value = usize::from(read_u16_async(reader).await?);
            if value <= usize::from(u8::MAX) {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        0xce => {
            let value = usize::try_from(read_u32_async(reader).await?).map_err(|_| {
                foks_rpc::Error::FrameTooLarge {
                    received: usize::MAX,
                    maximum,
                }
            })?;
            if value <= usize::from(u16::MAX) {
                return Err(foks_rpc::Error::FrameLengthMarker(marker));
            }
            value
        }
        _ => return Err(foks_rpc::Error::FrameLengthMarker(marker)),
    };
    if length > maximum {
        return Err(foks_rpc::Error::FrameTooLarge {
            received: length,
            maximum,
        });
    }
    Ok(length)
}

async fn read_byte_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u8> {
    let mut byte = [0];
    reader.read_exact(&mut byte).await?;
    Ok(byte[0])
}

async fn read_u16_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes).await?;
    Ok(u16::from_be_bytes(bytes))
}

async fn read_u32_async<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_be_bytes(bytes))
}

pub fn phase_deadline(
    deadline: crate::deadline::Deadline,
    maximum: std::time::Duration,
) -> Result<tokio::time::Instant> {
    deadline.remaining()?;
    let phase = crate::deadline::Deadline::after(maximum)?;
    Ok(deadline.instant().min(phase.instant()).into())
}

pub async fn write_response_until<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    response: &[u8],
    deadline: crate::deadline::Deadline,
    maximum: std::time::Duration,
) -> Result<()> {
    tokio::time::timeout_at(phase_deadline(deadline, maximum)?, async {
        writer.write_all(response).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| crate::Error::Io(timeout_error("response deadline expired")))??;
    Ok(())
}

fn timeout_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        deadline::Deadline,
        memory::{test_support, Request},
    };
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::{io::AsyncWrite, sync::oneshot};

    #[tokio::test]
    async fn canonical_lengths_match_existing_rpc_decoder() {
        for length in [0usize, 1, 127, 128, 255, 256, 65535, 65536] {
            let mut frame =
                foks_snowpack::encode(&foks_snowpack::Value::Unsigned(length as u64)).unwrap();
            frame.resize(frame.len() + length, 0x61);
            let expected = foks_rpc::read_frame(&mut frame.as_slice(), 65536).unwrap();
            let mut input = frame.as_slice();
            let (actual, deadline) = read_frame_ingress(&mut input, 65536, Duration::from_secs(15))
                .await
                .unwrap();
            assert_eq!(actual, expected.len());
            assert_eq!(input, expected);
            assert!(deadline.remaining().unwrap() <= Duration::from_secs(15));
        }
        for frame in [
            vec![0xcc, 1],
            vec![0xcd, 0, 255],
            vec![0xce, 0, 0, 1, 0],
            vec![0xd0, 0x80],
            vec![0xcd, 4, 1],
        ] {
            let expected = foks_rpc::read_frame(&mut frame.as_slice(), 1024).unwrap_err();
            let actual = read_frame_ingress(&mut frame.as_slice(), 1024, Duration::from_secs(15))
                .await
                .unwrap_err();
            assert_eq!(actual.to_string(), expected.to_string());
        }
        let error = read_frame_ingress(&mut &[0xcd, 1][..], 1024, Duration::from_secs(15))
            .await
            .unwrap_err();
        assert!(
            matches!(error, foks_rpc::Error::Io(error) if error.kind() == std::io::ErrorKind::UnexpectedEof)
        );
    }

    #[tokio::test]
    async fn deadline_conversion_retains_errors_and_refuses_expired_output() {
        let error = read_frame_ingress(&mut &[0][..], 1, Duration::MAX)
            .await
            .unwrap_err();
        assert!(
            matches!(error, foks_rpc::Error::Io(error) if error.to_string() == "invalid server configuration: request deadline overflows monotonic clock")
        );
        let mut bytes = Vec::new();
        let error = write_response_until(
            &mut bytes,
            b"late",
            Deadline::after(Duration::ZERO).unwrap(),
            Duration::from_secs(15),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut && error.to_string() == "request deadline expired")
        );
        assert!(bytes.is_empty());
        let deadline = Deadline::after(Duration::from_secs(15)).unwrap();
        assert_eq!(
            phase_deadline(deadline, Duration::from_secs(30)).unwrap(),
            deadline.instant().into()
        );
        assert!(matches!(
            phase_deadline(deadline, Duration::MAX),
            Err(crate::Error::Config(
                "request deadline overflows monotonic clock"
            ))
        ));
    }

    struct FlushGate {
        written: Arc<AtomicUsize>,
        arrived: Option<oneshot::Sender<()>>,
        release: oneshot::Receiver<()>,
    }
    impl AsyncWrite for FlushGate {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.fetch_add(bytes.len(), Ordering::Relaxed);
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            if let Some(arrived) = self.arrived.take() {
                let _ = arrived.send(());
            }
            Pin::new(&mut self.release).poll(cx).map(|result| {
                result.map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "flush abandoned")
                })
            })
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.poll_flush(cx)
        }
    }
    struct Payload {
        bytes: Vec<u8>,
        _owner: Arc<Request>,
    }
    impl Payload {
        async fn transmit(self, mut writer: FlushGate) {
            write_response_until(
                &mut writer,
                &self.bytes,
                Deadline::after(Duration::from_secs(5)).unwrap(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn physical_flush_or_disposal_keeps_original_output_owner_until_completion() {
        for cancel in [false, true] {
            let pools = test_support::pools(test_support::Capacities {
                working: 128 * 1024,
                small: 0,
                output: 12 * 1024,
                control: 16384,
                bulk: 4,
            })
            .unwrap();
            let initial = test_support::snapshot(&pools);
            let request = pools.request(None);
            let weak = Arc::downgrade(&request);
            let value =
                foks_snowpack::encode(&foks_snowpack::Value::Binary(vec![7; 4096])).unwrap();
            let bytes = request
                .sync(|| foks_rpc::encode_success_response_at(&value, 0))
                .unwrap();
            let expected = bytes.len();
            let payload = Payload {
                bytes,
                _owner: request,
            };
            let written = Arc::new(AtomicUsize::new(0));
            let (arrived, ready) = oneshot::channel();
            let (release, gate) = oneshot::channel();
            let task = tokio::spawn(payload.transmit(FlushGate {
                written: written.clone(),
                arrived: Some(arrived),
                release: gate,
            }));
            tokio::time::timeout(Duration::from_secs(5), ready)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(written.load(Ordering::Relaxed), expected);
            assert!(!task.is_finished());
            assert!(weak.upgrade().is_some());
            assert!(test_support::snapshot(&pools).output < initial.output);
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                release.send(()).unwrap();
                task.await.unwrap();
            }
            assert!(weak.upgrade().is_none());
            assert_eq!(test_support::snapshot(&pools), initial);
        }
    }
}
