use std::io;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn respond_to_connection(mut stream: TcpStream) -> io::Result<()> {
    loop {
        stream.read(&mut [0; 128])?;
        stream.write_all(b"+PONG\r\n")?;
    }
}

fn main() -> io::Result<()> {
    println!("Logs from your program will appear here!");

    let listener = TcpListener::bind("127.0.0.1:6379")?;
    let mut handles = Vec::new();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                handles.push(thread::spawn(move || -> io::Result<()> {
                    respond_to_connection(stream)?;

                    Ok(())
                }));
            },
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }
    }
    
    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}
