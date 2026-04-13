use crate::resp;
use crate::resp::build::ErrorType;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/* Structs */

#[derive(Debug)]
enum Value {
    STRING(String),
    LIST(Vec<String>),
    STREAM(Stream),
}

#[derive(Debug)]
enum Time {
    VAR,
    FIX(Duration, Instant),
}

#[derive(Debug)]
pub struct ValueEntry {
    value: Value,
    time: Time
}

/* Type aliases */

pub type StreamID = (i64, i64);
pub type Stream = Vec<(StreamID, Vec<String>)>;
pub type DataTable = HashMap<String, ValueEntry>;
type BlockTable = HashMap<String, Arc<(Mutex<bool>, Condvar)>>;

/* Globals */

const NULL_BULK_STR: &[u8] = b"$-1\r\n";
const NULL_BULK_ARRAY: &[u8] = b"*-1\r\n";
static BLOCK_SET: OnceLock<Arc<Mutex<BlockTable>>> = OnceLock::new();

/* General helpers */

// Global block list.

fn init_block(targets: &[&str], timeout: Duration) -> bool {
    let block = get_block_set();
    let mut table = block.lock().unwrap();
    let cond = Arc::new((Mutex::new(false), Condvar::new()));

    // Add so other clients can't block on it.
    for &target in targets {
        (&mut table).insert(String::from(target), Arc::clone(&cond));
    }
    // Unblock table while waiting so other threads can
    // update it.
    drop(table);

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
            started = s;
            if time_result.timed_out() {
                return false;
            }
        }
    }

    // Blocks should be removed by the caller.
    true
}

fn get_block_set() -> Arc<Mutex<BlockTable>> {
    let set = BLOCK_SET.get_or_init(|| {
        Arc::new(Mutex::new(BlockTable::new()))
    });

    Arc::clone(set)
}

fn block_exists(targets: &[&str]) -> bool {
    let block = get_block_set();
    let table = block.lock().unwrap();
    for &target in targets {
        if table.get(target).is_some() {
            return true;
        }
    }

    false
}

// Notifies the idle thread that the blocking condition
// has been met to wake it up.
fn release_block(target: &str) {
    let block = get_block_set();
    let mut guard = block.lock().unwrap();
    match guard.get_mut(target) {
        Some(var) => {
            let mut mutex = (&var.0).lock().unwrap();
            *mutex = true;
            var.1.notify_one();
        },
        None => {}
    }

    // Does nothing if target was not found.
    guard.remove(target);
}

// Removes the target block from the block list.
fn remove_block(target: &str) {
    let block = get_block_set();
    let mut table = block.lock().unwrap();
    (&mut table).remove(target);
}

fn parse_timeout(string: &str, secs: bool) -> Duration {
    if secs { // Input is in seconds (e.g., 0.5s).
        string.parse::<f32>().ok()
            .map(|time| Duration::from_millis((time * 1000.0) as u64))
            .unwrap_or(Duration::ZERO)
    } else { // Input is in milliseconds.
        string.parse::<u64>().ok()
            .map(|time| Duration::from_millis(time))
            .unwrap_or(Duration::ZERO)
    }
}

/* LPUSH/RPUSH */

fn prepare_entries(entries: &[String], reverse: bool) -> Vec<String> {
    let mut list = Vec::from(entries);
    if reverse {
        list.reverse();
    }

    list
}

/* LRANGE */

fn normalize_range_index(index: &mut i64, length: usize) {
    if *index < 0 {
        *index += length as i64;

        if *index < 0 {
            *index = 0;
        }
    }
}

/* XADD */

fn parse_stream_id(id: &str) -> StreamID {
    if id.starts_with('*') {
        return (-1, -1);
    }

    let pos = id.find('-').unwrap();
    let first: i64 = id[..pos].parse().unwrap();
    let second: i64 = if id.ends_with('*') {
        -1
    } else {
        id[pos + 1..].parse().unwrap()
    };

    (first, second)
}

/* XRANGE/XREAD */

fn parse_range_id(id_str: &str, default: i64) -> StreamID {
    if id_str.starts_with('+') {
        return (i64::MAX, i64::MAX);
    } else if id_str.starts_with('$') {
        return (-1, -1);
    }

    match id_str.find('-') {
        Some(pos) => {
            if pos == 0 {
                return (0, 0);
            }

            let time: i64 = id_str[..pos].parse().unwrap();
            let seq: i64 = id_str[pos + 1..].parse().unwrap();

            (time, seq)
        },
        None => (id_str.parse::<i64>().unwrap(), default),
    }
}

fn id_in_range(id: &StreamID, start: &StreamID, end: &StreamID) -> bool {
    if id.0 < start.0 || (id.0 == start.0 && id.1 < start.1) {
        return false;
    }

    if id.0 > end.0 || (id.0 == end.0 && id.1 > end.1) {
        return false;
    }

    true
}

fn validate_stream_id(
    response: &mut Vec<u8>,
    previous: &StreamID,
    new: &StreamID
) -> bool {
    const ZERO_ERR_MSG: &str =
        "The ID specified in XADD must be greater than 0-0";
    const SMALL_ERR_MSG: &str = "The ID specified in XADD is equal or \
        smaller than the target stream top item";

    if new.0 <= previous.0
        && (new.0 != previous.0 || new.1 <= previous.1) {
        *response = resp::build::resp_error(
            ErrorType::ERR,
            if new.0 == 0 && new.1 == 0 {
                ZERO_ERR_MSG
            } else {
                SMALL_ERR_MSG
            }
        );

        return false;
    }

    true
}

fn generate_stream_id(
    id_pair: &StreamID,
    previous: &StreamID
) -> (StreamID, Vec<u8>) {
    let mut num_pair: (i64, i64) = (0, 0);

    if id_pair.0 != -1 && id_pair.1 != -1 {
        num_pair = *id_pair;
    } else {
        num_pair.0 = if id_pair.0 == -1 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH).unwrap()
                .as_millis() as i64
        } else {
            id_pair.0
        };

        if num_pair.0 == previous.0 {
            num_pair.1 = previous.1 + 1;
        } else if num_pair.0 != 0 {
            num_pair.1 = 0;
        } else {
            num_pair.1 = 1;
        }
    }

    let mut pair_str = String::new();
    pair_str.push_str(&num_pair.0.to_string());
    pair_str.push('-');
    pair_str.push_str(&num_pair.1.to_string());

    (num_pair, resp::build::resp_bulk_str(&pair_str))
}

fn normalize_range_entries(
    stream_pairs: &mut Vec<(String, StreamID)>,
    store: &Arc<Mutex<DataTable>>
) {
    let guard = store.lock().unwrap();
    for (name, id) in stream_pairs {
        if id.0 != -1 && id.1 != -1 {
            continue;
        }

        if let Some(entry) = guard.get(name) {
            if let Value::STREAM(stream) = &entry.value {
                let max = stream.last().unwrap().0;
                *id = max; // Will be incremented in in_range_entries().
            }
        }
    }
}

fn in_range_entries(
    stream_pairs: &Vec<(String, StreamID)>,
    store: &Arc<Mutex<DataTable>>
) -> Vec<(String, Stream)> {
    let mut stream_array = Vec::new();
    let guard = store.lock().unwrap();
    for pair in stream_pairs {
        if let Some(entry) = guard.get(&pair.0) {
            if let Value::STREAM(stream) = &entry.value {
                // Exclusive, so minimum is 1 sequence higher than
                // the ID provided.
                let min = (pair.1.0, pair.1.1 + 1);
                let valid_entries: Stream = stream.iter().cloned()
                    .filter(
                        // No actual maximum.
                        |x| id_in_range(&x.0, &min, &(i64::MAX, i64::MAX))
                    ).collect();
                stream_array.push((pair.0.clone(), valid_entries));
            }
        }
    }

    stream_array
}

/* Main handlers */

fn handle_blpop(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let name = &arguments[1];
    if block_exists(&[name]) {
        return Ok(());
    }

    let mut response = Vec::from(NULL_BULK_ARRAY);
    let timeout = parse_timeout(&arguments[2], true);
    if init_block(&[name], timeout) {
        if let Some(entry) = store.lock().unwrap().get_mut(name) {
            if let Value::LIST(list) = &mut entry.value {
                let entries = [&name[..], &list.remove(0)];
                response = resp::build::resp_array(&entries);
            }
        }
    }

    // Unblock so new clients can (now) block on it.
    remove_block(name);
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

    release_block(&arguments[1]);
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
    let mut count: usize = 1;

    if arguments.len() > 2 {
        count = (&arguments[2]).parse().unwrap();
    }

    match store.lock().unwrap().get_mut(&arguments[1]) {
        Some(entry) => {
            if let Value::LIST(list) = &mut entry.value {
                if list.len() == 0 {
                    response = Vec::from(NULL_BULK_STR);
                } else if count == 1 {
                    let popped = list.remove(0);
                    response = resp::build::resp_bulk_str(&popped);
                } else {
                    if count > list.len() {
                        count = list.len();
                    }

                    let popped: Vec<&str> = (&list[..count]).iter().map(
                        |s: &String| &s[..]
                    ).collect();
                    response = resp::build::resp_array(&popped);
                    *list = Vec::from(&list[count..]);
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

    if arguments.len() == 5
        && (arguments.contains(&String::from("PX"))
        || arguments.contains(&String::from("EX"))) {
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

fn handle_type(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let response: &str;

    match store.lock().unwrap().get(&arguments[1]) {
        Some(entry) => {
            match entry.value {
                Value::STRING(_) => response = "string",
                Value::LIST(_) =>   response = "list",
                Value::STREAM(_) => response = "stream",
            }
        },
        None => response = "none",
    }

    let response = resp::build::resp_simple_str(response);
    stream.write_all(&response)?;
    Ok(())
}

fn handle_xadd(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let key = arguments[1].clone();
    let id = &arguments[2];
    let array = Vec::from(&arguments[3..]);
    let mut response: Vec<u8> = Vec::new();

    let mut guard = store.lock().unwrap();
    let mut id_pair = parse_stream_id(id);
    match guard.get_mut(&key) {
        Some(entry) => {
            if let Value::STREAM(stream) = &mut entry.value {
                let previous = &stream.last().unwrap().0;
                (id_pair, response) = generate_stream_id(&id_pair, previous);
                if validate_stream_id(&mut response, previous, &id_pair) {
                    stream.push((id_pair, array));
                }
            }
        },
        None => {
            (id_pair, response) = generate_stream_id(&id_pair, &(0, 0));
            if validate_stream_id(&mut response, &(0, 0), &id_pair) {
                let entries = Vec::from([(id_pair, array)]);
                guard.insert(key, ValueEntry {
                    value: Value::STREAM(entries),
                    time: Time::VAR
                });
            }
        }
    }

    release_block(&arguments[1]);
    stream.write_all(&response)?;
    Ok(())
}

fn handle_xrange(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let start_id = parse_range_id(&arguments[2], 0);
    let end_id = parse_range_id(&arguments[3], i64::MAX);
    let mut range_entries: Stream = Stream::new();

    if let Some(entry) = store.lock().unwrap().get(&arguments[1]) {
        if let Value::STREAM(stream) = &entry.value {
            range_entries = stream
                .iter()
                .cloned()
                .filter(|x| id_in_range(&x.0, &start_id, &end_id))
                    .collect();
        }
    }

    let response = resp::build::resp_stream_array(&range_entries);
    stream.write_all(&response)?;
    Ok(())
}

fn handle_xread(
    arguments: &Vec<String>,
    stream: &mut TcpStream,
    store: &Arc<Mutex<DataTable>>
) -> io::Result<()> {
    let block = (&arguments[1]).to_uppercase() == "BLOCK";
    let skip_args = if block { 4 } else { 2 };
    let stream_count = (arguments.len() - skip_args) / 2;
    let targets: Vec<&str> = (&arguments[skip_args..skip_args + stream_count])
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut stream_pairs: Vec<(String, StreamID)> = Vec::new();
    for i in 0..stream_count {
        stream_pairs.push((
            String::from(&arguments[skip_args + i]),
            parse_range_id(&arguments[skip_args + stream_count + i], 0)
        ));
    }

    normalize_range_entries(&mut stream_pairs, &store);
    let mut blocked_read_fail: bool = false;

    if block {
        if block_exists(&targets) {
            return Ok(());
        }

        let timeout = parse_timeout(&arguments[2], false);
        blocked_read_fail = !init_block(&targets, timeout);
    }

    targets.iter().for_each(|&target| remove_block(target));
    if blocked_read_fail {
        stream.write_all(&Vec::from(NULL_BULK_ARRAY))?;
    } else {
        let stream_array = in_range_entries(&stream_pairs, &store);
        let response = resp::build::resp_stream_multi_array(&stream_array);
        stream.write_all(&response)?;
    }
    Ok(())
}

/* Main driver */

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
        "TYPE" =>   handle_type(commands, stream, store)?,
        "XADD" =>   handle_xadd(commands, stream, store)?,
        "XRANGE" => handle_xrange(commands, stream, store)?,
        "XREAD" =>  handle_xread(commands, stream, store)?,
        _ => {
            return Ok(());
        }
    }

    Ok(())
}