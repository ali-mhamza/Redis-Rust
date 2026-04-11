pub mod commands;
pub mod connect;
pub mod resp;

pub use commands::DataTable;
pub use commands::{Stream, StreamID};
use std::io;
use std::net::{TcpListener};

fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:6379")?;
    connect::handle_connections(&listener)?;

    Ok(())
}