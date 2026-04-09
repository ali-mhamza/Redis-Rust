use std::io;
use std::io::{Read, Write};
use std::net::TcpListener;

fn main() -> io::Result<()> {
    println!("Logs from your program will appear here!");

    let listener = TcpListener::bind("127.0.0.1:6379")?;

    match listener.accept() {
        Ok((mut stream, _)) => {
            loop {
                let Ok(_) = stream.read(&mut [0; 128]) else {
                    break;
                };
                
                stream.write_all(b"+PONG\r\n")?;
            }
        },
        Err(e) => {
            eprintln!("Error: {e}");
        }
    }

    Ok(())
}
