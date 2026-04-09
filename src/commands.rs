use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time;
use std::time::{Duration, Instant};
use crate::resp;
use crate::resp::build::resp_integer;

enum Value {
    STRING(String),
    LIST(Vec<String>),
}

enum Time {
    VAR,
    FIX(time::Duration, time::Instant),
}

pub struct ValueEntry {
    value: Value,
    time: Time
}

pub type Table = HashMap<String, ValueEntry>;
const NULL_BULK_STR: &[u8] = b"$-1\r\n";

fn handle_get(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();
    let mut guard = store.lock().unwrap();

    match (&mut guard).get(&commands[1]) {
        Some(entry) => {
            if let Value::STRING(value) = &entry.value {
                let now = Instant::now();

                if let Time::FIX(duration, instant) = entry.time
                    && now.duration_since(instant) > duration {
                    response = Vec::from(NULL_BULK_STR);
                    guard.remove(&commands[1]);
                } else {
                    response = resp::build::resp_bulk_str(value);
                }
            }
        },
        None => {
            response = Vec::from(NULL_BULK_STR);
        },
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
            value: Value::STRING(commands[2].clone()),
            time: Time::FIX(Duration::from_millis(time), Instant::now())
        });
    } else {
        store.lock().unwrap().insert(commands[1].clone(), ValueEntry {
            value: Value::STRING(commands[2].clone()),
            time: Time::VAR
        });
    }

    response = resp::build::resp_simple_str("OK");
    stream.write_all(&response)?;
    Ok(())
}

fn handle_rpush(
    commands: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();
    let mut guard = store.lock().unwrap();

    match (&mut guard).get_mut(&commands[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &mut entry.value {
                list.push(commands[2].clone());
                dbg!(list.len());
                response = resp_integer(list.len() as i64);
            }
        },
        None => {
            guard.insert(commands[1].clone(), ValueEntry {
                value: Value::LIST(vec![]),
                time: Time::VAR
            });

            response = resp_integer(1);
        }
    }

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
        "PING" => {
            response = resp::build::resp_simple_str("PONG");
            stream.write_all(&response)?;
        },
        "RPUSH" => {
            handle_rpush(commands, stream, store)?;
        },
        "SET" => {
            handle_set(commands, stream, store)?;
        },
        _ => {
            return Ok(());
        }
    }

    Ok(())
}