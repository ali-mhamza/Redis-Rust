use crate::resp;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

enum Value {
    STRING(String),
    LIST(Vec<String>),
}

enum Time {
    VAR,
    FIX(Duration, Instant),
}

pub struct ValueEntry {
    value: Value,
    time: Time
}

pub type DataTable = HashMap<String, ValueEntry>;
type BlockTable = HashMap<String, Arc<(Mutex<bool>, Condvar)>>;

const NULL_BULK_STR: &[u8] = b"$-1\r\n";
const NULL_BULK_ARRAY: &[u8] = b"*-1\r\n";

static BLOCK_SET: OnceLock<Arc<Mutex<BlockTable>>> = OnceLock::new();

fn get_block_set() -> Arc<Mutex<BlockTable>> {
    let set = BLOCK_SET.get_or_init(|| {
        Arc::new(Mutex::new(BlockTable::new()))
    });

    Arc::clone(set)
}

fn handle_blpop(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let mut response = Vec::from(NULL_BULK_ARRAY);
    let name = &arguments[1];
    let timeout = arguments
        .get(2)
        .and_then(|s| s.parse::<f32>().ok())
        .map(|secs| Duration::from_millis((secs * 1000.0) as u64))
        .unwrap_or(Duration::ZERO);

    let block = get_block_set();
    let mut table = block.lock().unwrap();
    if table.get(name).is_some() {
        return Ok(())
    }

    let cond = Arc::new((Mutex::new(false), Condvar::new()));
    // Add so other clients can't block on it.
    (&mut table).insert(name.clone(), Arc::clone(&cond));

    let (lock, cvar) = &*cond;
    let mut started = lock.lock().unwrap();
    if timeout.is_zero() {
        while !*started {
            started = cvar.wait(started).unwrap();
        }
    } else {
        while !*started {
            let (s, time_result) = cvar
                .wait_timeout(started, timeout)
                .unwrap();
            if time_result.timed_out() {
                break;
            }
            started = s;
        }
    }

    if let Some(entry) = store.lock().unwrap().get_mut(name) {
        if let Value::LIST(list) = &mut entry.value {
            let entries = [&name[..], &list.remove(0)];
            response = resp::build::resp_array(&entries);
        }
    }

    // Remove so new clients can (now) block on it.
    (&mut table).remove(name);
    stream.write_all(&response)?;
    Ok(())
}

fn handle_get(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
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
    store: &Arc<Mutex<DataTable>>
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

fn check_blocks(target: &str) {
    let block = get_block_set();
    match block.lock().unwrap().get_mut(target) {
        Some(var) => {
            let mut mutex = (&var.0).lock().unwrap();
            *mutex = true;
            var.1.notify_one();
        },
        None => {}
    }
}

fn handle_list_push(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>,
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

    check_blocks(&arguments[1]);
    stream.write_all(&response)?;
    Ok(())
}

fn handle_llen(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
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
    store: &Arc<Mutex<DataTable>>
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
    store: &Arc<Mutex<DataTable>>
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
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let cmd = commands[0].to_uppercase();
    let response: Vec<u8>;

    match &cmd[..] {
        "BLPOP" =>  handle_blpop(commands, stream, store)?,
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