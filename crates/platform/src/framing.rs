//! Reading newline-delimited messages with a bound on how much is buffered and how long it may take (BUILD-005,
//! findings F04 and F05). Used by the UI control channel and the MCP server, which both read untrusted lines.
//!
//! A line is never buffered past its limit: the reader stops at `max` + 1 bytes and reports [`LineRead::TooLong`],
//! and [`skip_line`] then throws away the rest of that line without keeping it, so the next request is read from
//! the right place and one bad line does not end the session.

/// What [`read_line_limited`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum LineRead {
    /// A line, now in the buffer (newline and a trailing `\r` stripped).
    Line,
    /// End of stream.
    End,
    /// The line is longer than the limit; at most limit + 1 bytes of it were read.
    TooLong,
}

/// Read one `\n`-terminated line of at most `max` bytes into `buf`, giving up with `TimedOut` once `deadline` has
/// passed. The deadline is absolute, so a client that sends a byte at a time cannot keep the read open past it; a
/// socket whose read timeout is short (see `READ_TICK` in the control channel) wakes often enough to notice.
pub fn read_line_limited(
    reader: &mut impl std::io::BufRead,
    buf: &mut Vec<u8>,
    max: usize,
    deadline: Option<std::time::Instant>,
) -> std::io::Result<LineRead> {
    use std::io::ErrorKind;
    buf.clear();
    loop {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Err(std::io::Error::new(ErrorKind::TimedOut, "the read deadline passed"));
        }
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e),
        };
        if chunk.is_empty() {
            // End of stream: a last line without a newline is still a line.
            return Ok(if buf.is_empty() { LineRead::End } else { LineRead::Line });
        }
        // Never buffer more than `max` + 1 bytes for one line.
        let room = (max + 1).saturating_sub(buf.len());
        let window = &chunk[..chunk.len().min(room)];
        if let Some(at) = window.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&window[..at]);
            reader.consume(at + 1);
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(LineRead::Line);
        }
        let taken = window.len();
        buf.extend_from_slice(window);
        reader.consume(taken);
        if buf.len() > max {
            return Ok(LineRead::TooLong);
        }
    }
}

/// Read and throw away up to the end of the current line, without keeping it. `false` if the line is longer than
/// `limit` bytes, the deadline passes or the stream ends first.
pub fn skip_line(reader: &mut impl std::io::BufRead, limit: u64, deadline: std::time::Instant) -> bool {
    use std::io::ErrorKind;
    let mut skipped = 0u64;
    loop {
        if std::time::Instant::now() >= deadline || skipped > limit {
            return false;
        }
        let chunk = match reader.fill_buf() {
            Ok([]) => return false,
            Ok(chunk) => chunk,
            Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(_) => return false,
        };
        match chunk.iter().position(|&b| b == b'\n') {
            Some(at) => {
                reader.consume(at + 1);
                return true;
            }
            None => {
                let n = chunk.len();
                skipped += n as u64;
                reader.consume(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_read_up_to_the_limit() {
        let mut buf = Vec::new();
        let mut input: &[u8] = b"aaaaaaaaaa
bb
cc";
        assert_eq!(read_line_limited(&mut input, &mut buf, 10, None).unwrap(), LineRead::Line);
        assert_eq!(buf, b"aaaaaaaaaa", "a line exactly at the limit is accepted");
        assert_eq!(read_line_limited(&mut input, &mut buf, 10, None).unwrap(), LineRead::Line);
        assert_eq!(buf, b"bb", "CRLF is stripped");
        assert_eq!(read_line_limited(&mut input, &mut buf, 10, None).unwrap(), LineRead::Line);
        assert_eq!(buf, b"cc", "a last line without a newline is still read");
        assert_eq!(read_line_limited(&mut input, &mut buf, 10, None).unwrap(), LineRead::End, "then end of stream");
        let long = [b'x'; 100];
        let mut input: &[u8] = &long;
        assert_eq!(read_line_limited(&mut input, &mut buf, 10, None).unwrap(), LineRead::TooLong);
        assert_eq!(input.len(), 100 - 11, "at most limit + 1 bytes are read; the rest is left");
        assert!(buf.len() <= 11, "never more than limit + 1 bytes are buffered: {}", buf.len());
    }

    #[test]
    fn an_over_long_line_is_skipped_and_the_next_one_is_intact() {
        let mut data = vec![b'x'; 5000];
        data.extend_from_slice(
            b"
next
",
        );
        let mut input: &[u8] = &data;
        let mut buf = Vec::new();
        assert_eq!(read_line_limited(&mut input, &mut buf, 16, None).unwrap(), LineRead::TooLong);
        let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
        assert!(skip_line(&mut input, 1 << 20, far), "the end of the long line was found");
        assert_eq!(read_line_limited(&mut input, &mut buf, 16, None).unwrap(), LineRead::Line);
        assert_eq!(buf, b"next");
    }

    #[test]
    fn skipping_gives_up_on_a_line_with_no_end_within_its_limit() {
        let data = vec![b'x'; 100_000];
        let mut input: &[u8] = &data;
        let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
        assert!(!skip_line(&mut input, 1000, far), "a line longer than the skip limit is not followed forever");
        let mut empty: &[u8] = b"";
        assert!(!skip_line(&mut empty, 1000, far), "end of stream before a newline");
    }

    #[test]
    fn a_passed_deadline_is_a_timeout() {
        let mut buf = Vec::new();
        let mut input: &[u8] = b"data
";
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let e = read_line_limited(&mut input, &mut buf, 10, Some(past)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
    }
}
