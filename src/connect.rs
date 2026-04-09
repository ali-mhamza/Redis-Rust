use crate::parse;

use std::{io, thread};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::str;

const BUFFER_SIZE: usize = 1024;

fn handle_command(commands: &Vec<String>, stream: &mut TcpStream) -> io::Result<()> {
    let cmd = commands[0].to_uppercase();
    match &cmd[..] {
        "ECHO" => {
            stream.write_all(commands[1].as_bytes())?;
        },
        "PING" => {
            stream.write_all(b"+PONG\r\n")?;
        },
        _ => {
            return Ok(());
        }
    }
    
    Ok(())
}

fn respond_to_connection(mut stream: TcpStream) -> io::Result<()> {
    loop {
        let mut buf = [0; BUFFER_SIZE];
        stream.read(&mut buf)?;
        let string = str::from_utf8(&buf).unwrap();
        
        let commands = parse::parse_resp_array(string);
        handle_command(&commands, &mut stream)?;
    }
}

pub fn handle_connections(listener: &TcpListener) -> io::Result<()> {
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
        let _ = handle.join().unwrap();
    }

    Ok(())
}