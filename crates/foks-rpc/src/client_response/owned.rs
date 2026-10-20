//! Consuming success framing with one final allocation and no content copy.
use super::{
    encode_text, encode_unsigned, Error, Result, DEFAULT_MAX_FRAME_LENGTH, METHOD_RESPONSE,
    RESPONSE_HEADER,
};

/// Consume a complete response and encode the existing success envelope directly
/// into the final frame. The borrowed encoder remains the compatibility oracle.
/// Admission still retains the original conservative two-buffer reservation.
pub fn encode_success_response_owned_at(response: Vec<u8>, sequence: u64) -> Result<Vec<u8>> {
    encode_with_reservation(response, sequence, |output, length| {
        output
            .try_reserve_exact(length)
            .map_err(|_| Error::Io(std::io::Error::from(std::io::ErrorKind::OutOfMemory)))
    })
}
fn unsigned_length(value: u64) -> usize {
    match value {
        0..=0x7f => 1,
        0x80..=0xff => 2,
        0x100..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}
fn encode_with_reservation(
    response: Vec<u8>,
    sequence: u64,
    reserve: impl FnOnce(&mut Vec<u8>, usize) -> Result<()>,
) -> Result<Vec<u8>> {
    // Preserve the borrowed API's admission, validation and size-error order.
    crate::response_budget::reserve_success(response.len())?;
    foks_snowpack::validate(&response)?;
    let overflow = || Error::FrameTooLarge {
        received: usize::MAX,
        maximum: DEFAULT_MAX_FRAME_LENGTH,
    };
    // Array + method + sequence + nil + map + encoded Data/Header keys + header.
    let overhead = 1
        + unsigned_length(METHOD_RESPONSE)
        + unsigned_length(sequence)
        + 1
        + 1
        + 5
        + 7
        + RESPONSE_HEADER.len();
    let content_length = response.len().checked_add(overhead).ok_or_else(overflow)?;
    if content_length > DEFAULT_MAX_FRAME_LENGTH {
        return Err(Error::FrameTooLarge {
            received: content_length,
            maximum: DEFAULT_MAX_FRAME_LENGTH,
        });
    }
    let final_length = content_length
        .checked_add(unsigned_length(content_length as u64))
        .ok_or_else(overflow)?;
    let mut output = Vec::new();
    reserve(&mut output, final_length)?;
    encode_unsigned(content_length as u64, &mut output);
    output.push(0x94);
    encode_unsigned(METHOD_RESPONSE, &mut output);
    encode_unsigned(sequence, &mut output);
    output.push(0xc0);
    output.push(0x82);
    encode_text(b"Data", &mut output);
    output.extend_from_slice(&response);
    encode_text(b"Header", &mut output);
    output.extend_from_slice(RESPONSE_HEADER);
    debug_assert_eq!(output.len(), final_length);
    drop(response);
    Ok(output)
}

#[cfg(test)]
mod tests;
