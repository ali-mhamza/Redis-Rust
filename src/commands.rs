use crate::resp;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time;
use std::time::{Duration, Instant};

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

static BLOCK_SET: OnceLock<Arc<Mutex<HashSet<String>>>> = OnceLock::new();

fn handle_blpop(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    const BLOCK_SLEEP_TIME: Duration = Duration::from_millis(500);

    let name = &arguments[1];
    // Getting but ignoring timeout for now.
    let _timeout: i64 = if arguments.len() > 2 {
        (&arguments[2]).parse().unwrap()
    } else { 0 };

    let block = Arc::clone(BLOCK_SET.get().unwrap());
    let mut guard = block.lock().unwrap();
    let None = guard.get(name) else {
        return Ok(())
    };

    // Add so other clients can't block on it.
    (&mut guard).insert(name.clone());

    loop {
        match store.lock().unwrap().get(name) {
            Some(entry) => {
                if let Value::LIST(list) = &entry.value
                    && list.len() != 0 {
                    let response = resp::build::resp_array(
                        &[&name[..], &list[0][..]]
                    );
                    stream.write_all(&response)?;
                    break;
                }
            },
            None => {}
        }

        thread::sleep(BLOCK_SLEEP_TIME);
    }

    // Remove so new clients can (now) block on it.
    (&mut guard).remove(name);
    Ok(())
}

fn handle_get(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();
    let mut guard = store.lock().unwrap();

    match (&mut guard).get(&arguments[1]) {
        Some(entry) => {
            if let Value::STRING(value) = &entry.value {
                let now = Instant::now();

                if let Time::FIX(duration, instant) = entry.time
                    && now.duration_since(instant) > duration {
                    response = Vec::from(NULL_BULK_STR);
                    guard.remove(&arguments[1]);
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

fn normalize_range_index(index: &mut i64, length: usize) {
    if *index < 0 {
        *index += length as i64;

        if *index < 0 {
            *index = 0;
        }
    }
}

fn handle_lrange(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut slices: Vec<&str> = Vec::new();
    let guard = store.lock().unwrap();

    match guard.get(&arguments[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &entry.value {
                let mut start: i64 = (&arguments[2]).parse().unwrap();
                let mut end: i64 = (&arguments[3]).parse().unwrap();

                [&mut start, &mut end].iter_mut()
                    .for_each(|i| normalize_range_index(*i, list.len()));
                let (start, mut end): (usize, usize) = (start as usize, end as usize);

                if start >= list.len() || start > end {
                    slices = vec![];
                } else {
                    if end >= list.len() {
                        end = list.len() - 1;
                    }

                    slices = (&list[start..=end]).iter().map(
                        |s: &String| &s[..]
                    ).collect();
                }
            }
        },
        None => {
            slices = vec![];
        }
    }

    let response = resp::build::resp_array(&slices);
    stream.write_all(&response)?;
    Ok(())
}

fn prepare_entries(entries: &[String], reverse: bool) -> Vec<String> {
    let mut list = Vec::from(entries);
    if reverse {
        list.reverse();
    }

    list
}

fn handle_list_push(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>,
    reverse: bool
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();
    let mut guard = store.lock().unwrap();
    let mut entries = prepare_entries(&arguments[2..], reverse);

    match (&mut guard).get_mut(&arguments[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &mut entry.value {
                if reverse {
                    list.splice(0..0, entries);
                } else {
                    list.append(&mut entries);
                }
                response = resp::build::resp_integer(list.len() as i64);
            }
        },
        None => {
            let length = entries.len();
            guard.insert(arguments[1].clone(), ValueEntry {
                value: Value::LIST(entries),
                time: Time::VAR
            });

            response = resp::build::resp_integer(length as i64);
        }
    }

    stream.write_all(&response)?;
    Ok(())
}

fn handle_llen(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();

    match store.lock().unwrap().get(&arguments[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &entry.value {
                response = resp::build::resp_integer(list.len() as i64);
            }
        },
        None => {
            response = resp::build::resp_integer(0);
        }
    }

    stream.write_all(&response)?;
    Ok(())
}

fn handle_lpop(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let mut response: Vec<u8> = Vec::new();
    let mut start: usize = 1;

    if arguments.len() > 2 {
        start = (&arguments[2]).parse().unwrap();
    }
    if start > arguments.len() {
        start = arguments.len();
    }

    match store.lock().unwrap().get_mut(&arguments[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &mut entry.value {
                if list.len() == 0 {
                    response = Vec::from(NULL_BULK_STR);
                } else if start == 1 {
                    let popped = list.remove(0);
                    response = resp::build::resp_bulk_str(&popped);
                } else {
                    let popped: Vec<&str> = (&list[..start]).iter().map(
                        |s: &String| &s[..]
                    ).collect();
                    response = resp::build::resp_array(&popped);
                    *list = Vec::from(&list[start..]);
                }
            }
        },
        None => {
            response = Vec::from(NULL_BULK_STR);
        }
    }

    stream.write_all(&response)?;
    Ok(())
}

fn handle_set(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<Table>>
) -> io::Result<()> {
    let response: Vec<u8>;

    if arguments.len() == 5 {
        // Only handling PX for now.
        let time: u64 = arguments[4].parse().unwrap();
        let option = arguments[3].to_uppercase();
        // Dummy value.
        let duration;

        if option == "EX" {
            duration = Duration::from_secs(time);
        } else { // PX
            duration = Duration::from_millis(time);
        }

        store.lock().unwrap().insert(arguments[1].clone(), ValueEntry {
            value: Value::STRING(arguments[2].clone()),
            time: Time::FIX(duration, Instant::now())
        });
    } else {
        store.lock().unwrap().insert(arguments[1].clone(), ValueEntry {
            value: Value::STRING(arguments[2].clone()),
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
        "GET" =>    handle_get(commands, stream, store)?,
        "LLEN" =>   handle_llen(commands, stream, store)?,
        "LPOP" =>   handle_lpop(commands, stream, store)?,
        "LPUSH" =>  handle_list_push(commands, stream, store, true)?,
        "LRANGE" => handle_lrange(commands, stream, store)?,
        "PING" => {
            response = resp::build::resp_simple_str("PONG");
            stream.write_all(&response)?;
        },
        "RPUSH" =>  handle_list_push(commands, stream, store, false)?,
        "SET" =>    handle_set(commands, stream, store)?,
        _ => {
            return Ok(());
        }
    }

    Ok(())
}