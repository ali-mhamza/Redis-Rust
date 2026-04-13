use crate::resp;
use std::io;
use std::io::Read;
use std::net::TcpStream;

const BUFFER_SIZE: usize = 1024;

pub fn read_input(stream: &mut TcpStream) -> io::Result<Vec<String>> {
    let mut buf = [0; BUFFER_SIZE];
    stream.read(&mut buf)?;
    let string = str::from_utf8(&buf).unwrap();

    let commands = resp::parse::parse_resp_array(string);
    Ok(commands)
}