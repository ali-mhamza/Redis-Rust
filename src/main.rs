pub mod commands;
pub mod connect;
pub mod resp;
pub mod utils;

pub use commands::{DataTable, Stream, StreamID};
use std::env;
use std::io;
use std::net::{TcpListener};

fn parse_port() -> i64 {
    let args: Vec<String> = env::args().collect();

    if args.len() == 3 {
        args[2].parse::<i64>().unwrap()
    } else {
        6379
    }
}

fn main() -> io::Result<()> {
    let addr = format!("127.0.0.1:{}", parse_port());
    let listener = TcpListener::bind(addr)?;
    connect::handle_connections(&listener)?;

    Ok(())
}