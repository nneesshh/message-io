use message_io::network::{NetEvent, Transport};
use message_io::node::{self};
use net_packet::take_packet;

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self};
use std::time::{Duration, Instant};

const EXPECTED_BYTES: usize = 1024 * 1024 * 1024; // 1GB

/// Similar to [`MAX_INTERNET_PAYLOAD_LEN`] but for localhost instead of internet.
/// Localhost can handle a bigger MTU.
#[cfg(not(target_os = "macos"))]
pub const MAX_LOCAL_PAYLOAD_LEN: usize = 65535 - 20 - 8;

#[cfg(target_os = "macos")]
pub const MAX_LOCAL_PAYLOAD_LEN: usize = 9216 - 20 - 8;

const CHUNK: usize = MAX_LOCAL_PAYLOAD_LEN;

fn main() {
    println!("Sending 1GB in chunks of {} bytes:\n", CHUNK);
    throughput_message_io(Transport::Tcp, CHUNK);
    println!();
    throughput_native_tcp(CHUNK);
}

fn throughput_message_io(transport: Transport, packet_size: usize) {
    print!("message-io {}:  \t", transport);
    std::io::stdout().flush().unwrap();

    let (engine, handler) = node::split();
    let message = (0..packet_size).map(|_| 0xFF).collect::<Vec<u8>>();

    let (t_ready, r_ready) = mpsc::channel();
    let (t_time, r_time) = mpsc::channel();

    let handler2 = handler.clone();
    let _ = std::thread::spawn(move || {
        let (_listener_id, addr) = handler2.network().listen_sync(transport, "127.0.0.1:0").unwrap();
        let (endpoint, _) = handler2.network().connect_sync(transport, addr).unwrap();

        if transport.is_connection_oriented() {
            r_ready.recv().unwrap();
        }
    
        // Ensure that the connection is performed,
        // the internal thread is initialized for not oriented connection protocols
        // and we are waiting in the internal poll for data.
        std::thread::sleep(Duration::from_millis(100));
    
        let start_time = Instant::now();
        while handler2.is_running() {
            let mut buffer = take_packet(message.len());
            buffer.append_slice(message.as_slice());
            handler2.network().send(endpoint, buffer);
        }
    
        let end_time = r_time.recv().unwrap();
        let elapsed = end_time - start_time;
        println!("Throughput: {}", ThroughputMeasure(EXPECTED_BYTES, elapsed));
    });

    let mut task = {
        let mut received_bytes = 0;
        let handler3 = handler.clone();

        node::node_listener_for_each_async(engine, &handler, move |event| match event.network() {
            NetEvent::Connected(_, _) => (),
            NetEvent::Accepted(_, _) => t_ready.send(()).unwrap(),
            NetEvent::Message(_, data) => {
                received_bytes += data.peek().len();
                if received_bytes >= EXPECTED_BYTES {
                    handler3.stop();
                    t_time.send(Instant::now()).unwrap();
                }
            }
            NetEvent::Disconnected(_) => (),
        })
    };
    task.wait();
}

fn throughput_native_tcp(packet_size: usize) {
    print!("native Tcp: \t\t");
    io::stdout().flush().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let message = (0..packet_size).map(|_| 0xFF).collect::<Vec<u8>>();
    let mut buffer: [u8; CHUNK] = [0; CHUNK];

    let thread = {
        let message = message.clone();
        std::thread::Builder::new()
            .name("sender".into())
            .spawn(move || {
                let mut total_sent = 0;
                let mut sender = TcpStream::connect(addr).unwrap();

                let start_time = Instant::now();
                while total_sent < EXPECTED_BYTES {
                    sender.write(&message).unwrap();
                    total_sent += message.len();
                }
                start_time
            })
            .unwrap()
    };

    let (mut receiver, _) = listener.accept().unwrap();
    let mut total_received = 0;
    while total_received < EXPECTED_BYTES {
        total_received += receiver.read(&mut buffer).unwrap();
    }
    let end_time = Instant::now();

    let start_time = thread.join().unwrap();
    let elapsed = end_time - start_time;
    println!("Throughput: {}", ThroughputMeasure(EXPECTED_BYTES, elapsed));
}

pub struct ThroughputMeasure(usize, Duration); //bytes, elapsed
impl std::fmt::Display for ThroughputMeasure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes_per_sec = self.0 as f64 / self.1.as_secs_f64();
        if bytes_per_sec < 1000.0 {
            write!(f, "{:.2} B/s", bytes_per_sec)
        } else if bytes_per_sec < 1000_000.0 {
            write!(f, "{:.2} KB/s", bytes_per_sec / 1000.0)
        } else if bytes_per_sec < 1000_000_000.0 {
            write!(f, "{:.2} MB/s", bytes_per_sec / 1000_000.0)
        } else {
            write!(f, "{:.2} GB/s", bytes_per_sec / 1000_000_000.0)
        }
    }
}
