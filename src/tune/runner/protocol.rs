use super::{InvocationResult, RunnerFailure, TuneInvocation};

pub(super) fn parse(
    stdout: &[u8],
    invocation: &TuneInvocation,
    elapsed_ns: u64,
) -> Result<InvocationResult, RunnerFailure> {
    if stdout.len() > 4_096 {
        return Err(RunnerFailure::StdoutLimit);
    }
    let text = std::str::from_utf8(stdout).map_err(|_| RunnerFailure::Protocol)?;
    if !text.ends_with('\n') || text[..text.len() - 1].contains('\n') {
        return Err(RunnerFailure::Protocol);
    }
    let fields = text[..text.len() - 1].split(' ').collect::<Vec<_>>();
    if fields.len() != 6 || fields[0] != "CKTUNE/1" || fields[1] != invocation.case_id {
        return Err(RunnerFailure::Protocol);
    }
    let seed = fields[2]
        .parse::<u64>()
        .map_err(|_| RunnerFailure::Protocol)?;
    let iterations = fields[3]
        .parse::<u64>()
        .map_err(|_| RunnerFailure::Protocol)?;
    let completed = fields[4]
        .parse::<u64>()
        .map_err(|_| RunnerFailure::Protocol)?;
    if seed != invocation.seed
        || iterations != invocation.iterations
        || completed != invocation.iterations
    {
        return Err(RunnerFailure::Protocol);
    }
    let digest = decode_digest(fields[5]).ok_or(RunnerFailure::Protocol)?;
    if digest != invocation.expected_digest {
        return Err(RunnerFailure::Correctness);
    }
    Ok(InvocationResult {
        elapsed_ns,
        completed,
        digest,
    })
}

fn decode_digest(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Some(digest)
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}
