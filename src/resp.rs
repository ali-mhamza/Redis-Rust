pub mod parse {
    const SKIP_CRLF: usize = b"\r\n".len();

    fn parse_length(part: &str) -> usize {
        let mut pos: usize = 0;
        for byte in part.as_bytes() {
            if byte.is_ascii_digit() {
                pos += 1;
            } else {
                break;
            }
        }

        pos
    }

    pub fn parse_resp_string(input: &str) -> (String, usize) {
        // Skip the $ sign.
        let part = &input[1..];
        let mut pos = parse_length(part);

        let size: usize = part[..pos].parse().unwrap();
        // Skip the \r\n.
        pos += SKIP_CRLF;
        let text = &part[pos..pos + size];

        let string = String::from(text);
        (string, pos + size + SKIP_CRLF)
    }

    pub fn parse_resp_array(input: &str) -> Vec<String> {
        // Skip the * sign.
        let part = &input[1..];
        let mut pos = parse_length(part);
        let count: usize = part[..pos].parse().unwrap();

        let mut strings = Vec::with_capacity(count);

        pos += SKIP_CRLF + 1;
        for _ in 0..count {
            let (string, new_pos) = parse_resp_string(&input[pos..]);
            strings.push(string);
            pos += new_pos + 1;
        }

        strings
    }
}

pub mod build {
    const CRLF_BYTES: &[u8] = b"\r\n";

    pub fn resp_encode_str(string: &str) -> Vec<u8> {
        let mut vec = Vec::new();
        let size = string.len().to_string();
        let size = size.as_bytes();

        vec.push(b'$');
        vec.extend(size);
        vec.extend(CRLF_BYTES);
        vec.extend(string.as_bytes());
        vec.extend(CRLF_BYTES);

        vec
    }
}

#[cfg(test)]
mod test {
    use std::str;
    use crate::resp::parse::{parse_resp_array, parse_resp_string};

    #[test]
    fn test_parse_array() {
        let input = str::from_utf8(b"*2\r\n$4\r\nECHO\r\n$3\r\nhey\r\n")
            .unwrap();
        let vec = parse_resp_array(input);
        assert_eq!(vec, vec!["ECHO", "hey"]);
    }

    #[test]
    fn test_parse_string() {
        let input = str::from_utf8(b"$4\r\nPING\r\n")
            .unwrap();
        let string = parse_resp_string(input).0;
        assert_eq!(string, String::from("PING"));
    }
}