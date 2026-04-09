use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time;
use std::time::{Duration, Instant};
use crate::resp;

enum Time {
    VAR,
    FIX(time::Duration, time::Instant),
}

pub struct ValueEntry {
    value: String,
    time: Time
}

pub type Table = HashMap<String, ValueEntry>;
const NULL_BULK_STR: &[u8] = b"$-1\r\n";

fn handle_get(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let response: Vec<u8>;
    let mut remove = false;
    let mut guard = store.lock().unwrap();

    match guard.get(&commands[1]) {
        Some(entry) => {
            if let Time::FIX(duration, instant) = entry.time {
                let now = Instant::now();
                if now.duration_since(instant) > duration {
                    response = Vec::from(NULL_BULK_STR);
                    remove = true;
                } else {
                    response = resp::build::resp_bulk_str(&entry.value);
                }
            } else {
                response = resp::build::resp_bulk_str(&entry.value);
            }
        },
        None => {
            response = Vec::from(NULL_BULK_STR);
        },
    }

    if remove {
        guard.remove(&commands[1]);
    }
    stream.write_all(&response)?;

    Ok(())
}

fn handle_set(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let response: Vec<u8>;

    if commands.len() == 5 {
        // Only handling PX for now.
        let time: u64 = commands[4].parse().unwrap();
        store.lock().unwrap().insert(commands[1].clone(), ValueEntry {
            value: commands[2].clone(),
            time: Time::FIX(Duration::from_millis(time), Instant::now())
        });
    } else {
        store.lock().unwrap().insert(commands[1].clone(), ValueEntry {
            value: commands[2].clone(),
            time: Time::VAR
        });
    }

    response = resp::build::resp_simple_str("OK");
    stream.write_all(&response)?;
    Ok(())
}

pub fn handle_commands(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let cmd = commands[0].to_uppercase();
    let response: Vec<u8>;

    match &cmd[..] {
        "ECHO" => {
            response = resp::build::resp_bulk_str(&commands[1]);
            stream.write_all(&response)?;
        },
        "GET" => {
            handle_get(commands, stream, store)?;
        },
        "SET" => {
            handle_set(commands, stream, store)?;
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