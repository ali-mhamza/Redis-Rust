use std::io::{self, Write};
use std::net::TcpListener;

fn main() -> io::Result<()> {
    println!("Logs from your program will appear here!");

    let listener = TcpListener::bind("127.0.0.1:6379")?;

    loop {
        if let Ok((mut stream, _)) = listener.accept() {
            stream.write_all(b"+PONG\r\n")?;
        } else {
            break;
        }
    }

    Ok(())
}
