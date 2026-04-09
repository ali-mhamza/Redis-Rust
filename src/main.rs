pub mod connect;
pub mod parse;

use std::io;
use std::net::{TcpListener};

fn main() -> io::Result<()> {
    println!("Logs from your program will appear here!");

    let listener = TcpListener::bind("127.0.0.1:6379")?;
    connect::handle_connections(&listener)?;

    Ok(())
}
