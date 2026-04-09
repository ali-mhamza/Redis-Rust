use crate::resp;

use std::{io, thread};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::str;
use std::sync::{Arc, Mutex};

const BUFFER_SIZE: usize = 1024;
const NULL_BULK_STR: &[u8] = b"$-1\r\n";

fn handle_commands(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<HashMap<String, String>>>
) -> io::Result<()> {
    let cmd = commands[0].to_uppercase();
    let response: Vec<u8>;

    match &cmd[..] {
        "ECHO" => {
            response = resp::build::resp_bulk_str(&commands[1]);
            stream.write_all(&response)?;
        },
        "GET" => {
            match store.lock().unwrap().get(&commands[1]) {
                Some(value) => {
                    response = resp::build::resp_bulk_str(value);
                },
                None => {
                    response = Vec::from(NULL_BULK_STR);
                },
            }

            stream.write_all(&response)?;
        },
        "SET" => {
            store.lock().unwrap().insert(commands[1].clone(), commands[2].clone());
            response = resp::build::resp_simple_str("OK");
            stream.write_all(&response)?;
        },
        "PING" => {
            response = resp::build::resp_simple_str("PONG");
            stream.write_all(&response)?;
        },
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

fn respond_to_connection(
    mut stream: TcpStream,
    store: Arc<Mutex<HashMap<String, String>>>
) -> io::Result<()> {
    loop {
        let mut buf = [0; BUFFER_SIZE];
        stream.read(&mut buf)?;
        let string = str::from_utf8(&buf).unwrap();

        let commands = resp::parse::parse_resp_array(string);
        handle_commands(&commands, &mut stream, &store)?;
    }
}

pub fn handle_connections(listener: &TcpListener) -> io::Result<()> {
    let mut handles = Vec::new();
    let store = Arc::new(Mutex::new(HashMap::new()));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let store_clone = Arc::clone(&store);
                handles.push(thread::spawn(move || -> io::Result<()> {
                    respond_to_connection(stream, store_clone)?;

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