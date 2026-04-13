use crate::commands;
use crate::utils;
use crate::DataTable;

use std::{io, thread};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

fn respond_to_connection(
    mut stream: TcpStream,
    store: Arc<Mutex<DataTable>>
) -> io::Result<()> {
    loop {
        let commands = utils::read_input(&mut stream)?;
        commands::handle_command(&commands, &mut stream, &store)?;
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