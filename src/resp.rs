pub mod parse {
    const SKIP_CRLF: usize = b"\r\n".len();

    fn parse_digits(part: &str) -> usize {
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

    fn parse_resp_string(input: &str) -> (String, usize) {
        // Skip the $ sign.
        let part = &input[1..];
        let mut pos = parse_digits(part);

        let size: usize = part[..pos].parse().unwrap();
        // Skip the \r\n.
        pos += SKIP_CRLF;
        let text = &part[pos..pos + size];

        let string = String::from(text);
        (string, pos + size + SKIP_CRLF)
    }
    
    pub fn parse_resp_integer(input: &str) -> i64 {
        const SKIP_PREFIX: usize = b":_".len();

        // Skip the ':' and +/-.
        let size = parse_digits(&input[2..]);
        let integer: i64 = input[1..size + SKIP_PREFIX].parse().unwrap();

        integer
    }

    pub fn parse_resp_array(input: &str) -> Vec<String> {
        // Skip the * sign.
        let part = &input[1..];
        let mut pos = parse_digits(part);
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

    pub fn resp_bulk_str(string: &str) -> Vec<u8> {
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

    pub fn resp_simple_str(string: &str) -> Vec<u8> {
        let mut vec = Vec::new();
        vec.push(b'+');
        vec.extend(string.as_bytes());
        vec.extend(CRLF_BYTES);

        vec
    }

    pub fn resp_integer(value: i64) -> Vec<u8> {
        let mut vec = Vec::new();
        vec.push(b':');
        if value > 0 {
            vec.push(b'+');
        }
        let int_str = value.to_string();
        vec.extend(int_str.as_bytes());
        vec.extend(CRLF_BYTES);

        vec
    }

    pub fn resp_array(array: &[&str]) -> Vec<u8> {
        let mut vec = Vec::new();
        vec.push(b'*');
        let size_str = array.len().to_string();
        vec.extend(size_str.as_bytes());
        vec.extend(CRLF_BYTES);

        for string in array {
            vec.extend(resp_bulk_str(string));
        }

        vec
    }
}

#[cfg(test)]
mod test {
    use crate::resp::build::*;
    use crate::resp::parse::*;
    use std::str;

    #[test]
    fn test_parse_array() {
        let input = str::from_utf8(b"*2\r\n$4\r\nECHO\r\n$3\r\nhey\r\n")
            .unwrap();
        let vec = parse_resp_array(input);
        assert_eq!(vec, vec!["ECHO", "hey"]);

        let input = str::from_utf8(
            b"*5\r\n$3\r\nSET\r\n$3\r\nkey\r\n$5\r\nvalue\r\n$2\r\nEX\r\n$4\r\n1000\r\n"
        ).unwrap();
        let vec = parse_resp_array(input);
        assert_eq!(vec, vec!["SET", "key", "value", "EX", "1000"]);
    }

    #[test]
    fn test_parse_integer() {
        let input = resp_integer(10);
        let result = parse_resp_integer(str::from_utf8(&input).unwrap());
        assert_eq!(result, 10);
    }

    #[test]
    fn test_bulk_str() {
        let bulk_str = resp_bulk_str("hey");
        assert_eq!(bulk_str, Vec::from(b"$3\r\nhey\r\n"));
    }

    #[test]
    fn test_simple_str() {
        let simple_str = resp_simple_str("OK");
        assert_eq!(simple_str, Vec::from(b"+OK\r\n"));
    }

    #[test]
    fn test_integer() {
        let integer = resp_integer(10);
        assert_eq!(integer, Vec::from(b":+10\r\n"));

        let integer = resp_integer(-10);
        assert_eq!(integer, Vec::from(b":-10\r\n"));
    }

    #[test]
    fn test_array() {
        let array = ["a", "b", "c"];
        let result = resp_array(&array);

        assert_eq!(result, Vec::from(b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n"));

        let array: [&str; 0] = [];
        let result = resp_array(&array);

        assert_eq!(result, Vec::from(b"*0\r\n"));
    }
}