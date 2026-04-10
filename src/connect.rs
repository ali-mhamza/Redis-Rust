use crate::commands;
use crate::resp;
use crate::DataTable;

use std::{io, thread};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::str;
use std::sync::{Arc, Mutex};

const BUFFER_SIZE: usize = 1024;

fn respond_to_connection(
    mut stream: TcpStream,
    store: Arc<Mutex<DataTable>>
) -> io::Result<()> {
    loop {
        let mut buf = [0; BUFFER_SIZE];
        stream.read(&mut buf)?;
        let string = str::from_utf8(&buf).unwrap();

        let commands = resp::parse::parse_resp_array(string);
        commands::handle_commands(&commands, &mut stream, &store)?;
    }
}

pub fn handle_connections(listener: &TcpListener) -> io::Result<()> {
    let mut handles = Vec::new();
    let store = Arc::new(Mutex::new(DataTable::new()));

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