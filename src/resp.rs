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
    use crate::{Stream, StreamID};

    const CRLF_BYTES: &[u8] = b"\r\n";

    enum SizeType { STRING, ARRAY }
    pub enum ErrorType { ERR, WRONGTYPE, }

    fn resp_size(size: usize, size_type: SizeType) -> Vec<u8> {
        let mut vec = Vec::new();
        vec.push(match size_type {
            SizeType::STRING => b'$',
            SizeType::ARRAY =>  b'*'
        });

        let size_str = size.to_string();
        vec.extend(size_str.as_bytes());
        vec.extend(CRLF_BYTES);

        vec
    }

    pub fn resp_bulk_str(string: &str) -> Vec<u8> {
        let mut vec = Vec::new();

        vec.extend(resp_size(string.len(), SizeType::STRING));
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
        vec.extend(resp_size(array.len(), SizeType::ARRAY));

        for string in array {
            vec.extend(resp_bulk_str(string));
        }

        vec
    }

    pub fn resp_error(error: ErrorType, msg: &str) -> Vec<u8> {
        let mut vec = match error {
            ErrorType::ERR =>       Vec::from("-ERR "),
            ErrorType::WRONGTYPE => Vec::from("-WRONGTYPE "),
        };

        vec.extend(msg.as_bytes());
        vec.extend(CRLF_BYTES);

        vec
    }

    fn resp_stream_id(id: &StreamID) -> String {
        let mut id_str = String::new();

        let first_str = id.0.to_string();
        id_str.push_str(&first_str);
        id_str.push('-');
        let second_str = id.1.to_string();
        id_str.push_str(&second_str);

        id_str
    }

    pub fn resp_stream_array(stream: &Stream) -> Vec<u8> {
        let mut vec= Vec::new();
        const STREAM_PAIR_SIZE: &[u8] = b"*2\r\n";

        vec.extend(resp_size(stream.len(), SizeType::ARRAY));

        for pair in stream {
            vec.extend(STREAM_PAIR_SIZE);
            let id_str = resp_stream_id(&pair.0);
            vec.extend(resp_bulk_str(&id_str));

            vec.extend(resp_size(pair.1.len(), SizeType::ARRAY));

            for entry in &pair.1 {
                vec.extend(resp_bulk_str(entry));
            }
        }

        vec
    }
}

#[cfg(test)]
mod test {
    use crate::resp::build::*;
    use crate::resp::parse::*;
    use crate::Stream;
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

    #[test]
    fn test_error() {
        let error = "The ID specified in XADD must be greater than 0-0";
        let result = resp_error(ErrorType::ERR, error);

        assert_eq!(result, b"-ERR The ID specified in \
            XADD must be greater than 0-0\r\n");
    }

    #[test]
    fn test_stream_array() {
        let test_stream: Stream = Vec::from([
            ((1526985054069, 0), Vec::from([
                String::from("temperature"), String::from("36"),
                String::from("humidity"), String::from("95")
            ])),
            ((1526985054079, 0), Vec::from([
                String::from("temperature"), String::from("37"),
                String::from("humidity"), String::from("94")
            ]))
        ]);

        let result = resp_stream_array(&test_stream);
        let expect = b"*2\r\n\
            *2\r\n\
            $15\r\n1526985054069-0\r\n\
            *4\r\n\
            $11\r\ntemperature\r\n\
            $2\r\n36\r\n\
            $8\r\nhumidity\r\n\
            $2\r\n95\r\n\
            *2\r\n\
            $15\r\n1526985054079-0\r\n\
            *4\r\n\
            $11\r\ntemperature\r\n\
            $2\r\n37\r\n\
            $8\r\nhumidity\r\n\
            $2\r\n94\r\n";

        dbg!(result.len());
        dbg!(expect.len());
        println!("{:?}", String::from_utf8(result.clone()));
        println!("{:?}", String::from_utf8(expect.to_vec()));
        assert_eq!(result, expect);
    }
}