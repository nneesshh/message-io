use message_io::network::{NetEvent, Transport};
use message_io::node::{self, NodeEvent};
use message_io::util::thread::NamespacedThread;

use net_packet::take_packet;
use test_case::test_case;

use rand::{Rng, SeedableRng};

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

const LOCAL_ADDR: &'static str = "127.0.0.1:0";
const MIN_MESSAGE: &'static [u8] = &[42];
const BIG_MESSAGE_SIZE: usize = 1024 * 1024 * 8; // 8MB

lazy_static::lazy_static! {
    pub static ref TIMEOUT: Duration = Duration::from_secs(60);
    pub static ref TIMEOUT_SMALL: Duration = Duration::from_secs(1);
}

// Common error messages
const TIMEOUT_EVENT_RECV_ERR: &'static str = "Timeout, but an event was expected.";

mod util {
    use std::sync::Once;

    // Used to init the log only one time for all tests;
    static INIT: Once = Once::new();

    #[allow(dead_code)]
    pub enum LogThread {
        Enabled,
        Disabled,
    }

    #[allow(dead_code)]
    pub fn init_logger(log_thread: LogThread) {
        INIT.call_once(|| configure_logger(log_thread).unwrap());
    }

    fn configure_logger(log_thread: LogThread) -> Result<(), fern::InitError> {
        fern::Dispatch::new()
            .filter(|metadata| metadata.target().starts_with("message_io"))
            .format(move |out, message, record| {
                let thread_name = format!("[{}]", std::thread::current().name().unwrap());
                out.finish(format_args!(
                    "[{}][{}][{}]{} {}",
                    chrono::Local::now().format("%M:%S:%f"), // min:sec:nano
                    record.level(),
                    record.target().strip_prefix("message_io::").unwrap_or(record.target()),
                    if let LogThread::Enabled = log_thread { thread_name } else { String::new() },
                    message,
                ))
            })
            .chain(std::io::stdout())
            .apply()?;
        Ok(())
    }
}

#[allow(unused_imports)]
use util::LogThread;

fn start_echo_server(
    transport: Transport,
    expected_clients: usize,
) -> (NamespacedThread<()>, SocketAddr) {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let thread = NamespacedThread::spawn("test-server", move || {
        let mut messages_received = 0;
        let mut disconnections = 0;
        let mut clients = HashSet::new();

        let (node, listener) = node::split();

        let (listener_id, server_addr) = node.network().listen(transport, LOCAL_ADDR).unwrap();
        tx.send(server_addr).unwrap();

        listener.for_each(move |event| match event {
            NodeEvent::Waker(_) => panic!("{}", TIMEOUT_EVENT_RECV_ERR),
            NodeEvent::Network(net_event) => match net_event {
                NetEvent::Connected(..) => unreachable!(),
                NetEvent::Accepted(endpoint, id) => {
                    assert_eq!(listener_id, id);
                    match transport.is_connection_oriented() {
                        true => assert!(clients.insert(endpoint)),
                        false => unreachable!(),
                    }
                }
                NetEvent::Message(endpoint, data) => {
                    assert_eq!(MIN_MESSAGE, data.peek());

                    node.network().send(endpoint, data);

                    messages_received += 1;

                    if !transport.is_connection_oriented() {
                        // We assume here that if the protocol is not
                        // connection-oriented it will no create a resource.
                        // The remote will be managed from the listener resource
                        assert_eq!(listener_id, endpoint.resource_id());
                        if messages_received == expected_clients {
                            node.stop() //Exit from thread.
                        }
                    }
                }
                NetEvent::Disconnected(endpoint) => {
                    match transport.is_connection_oriented() {
                        true => {
                            disconnections += 1;
                            assert!(clients.remove(&endpoint));
                            if disconnections == expected_clients {
                                assert_eq!(expected_clients, messages_received);
                                assert_eq!(0, clients.len());
                                node.stop() //Exit from thread.
                            }
                        }
                        false => unreachable!(),
                    }
                }
            },
        });
    });

    let server_addr = rx.recv_timeout(*TIMEOUT).expect(TIMEOUT_EVENT_RECV_ERR);
    (thread, server_addr)
}

fn start_echo_client_manager(
    transport: Transport,
    server_addr: SocketAddr,
    clients_number: usize,
) -> NamespacedThread<()> {
    NamespacedThread::spawn("test-client", move || {
        let (node, listener) = node::split();

        let mut clients = HashSet::new();
        let mut received = 0;

        for _ in 0..clients_number {
            node.network().connect(transport, server_addr).unwrap();
        }

        listener.for_each(move |event| match event {
            NodeEvent::Waker(_) => panic!("{}", TIMEOUT_EVENT_RECV_ERR),
            NodeEvent::Network(net_event) => match net_event {
                NetEvent::Connected(server, status) => {
                    assert!(status);
                    let mut buffer = take_packet(MIN_MESSAGE.len());
                    buffer.append_slice(MIN_MESSAGE);
                    node.network().send(server, buffer);
                    assert!(clients.insert(server));
                }
                NetEvent::Message(endpoint, data) => {
                    assert!(clients.remove(&endpoint));
                    assert_eq!(MIN_MESSAGE, data.peek());
                    node.network().remove(endpoint.resource_id());

                    received += 1;
                    if received == clients_number {
                        node.stop(); //Exit from thread.
                    }
                }
                NetEvent::Accepted(..) => unreachable!(),
                NetEvent::Disconnected(_) => unreachable!(),
            },
        });
    })
}

//#[cfg_attr(feature = "tcp", test_case(Transport::Tcp, 1))]
#[cfg_attr(feature = "tcp", test_case(Transport::Tcp, 100))]
// NOTE: A medium-high `clients` value can exceeds the "open file" limits of an OS in CI
// with an obfuscated error message.
fn echo(transport: Transport, clients: usize) {
    //util::init_logger(LogThread::Enabled); // Enable it for better debugging

    let (_server_thread, server_addr) = start_echo_server(transport, clients);
    let _client_thread = start_echo_client_manager(transport, server_addr, clients);
}

#[cfg_attr(feature = "tcp", test_case(Transport::Tcp, BIG_MESSAGE_SIZE))]
fn message_size(transport: Transport, message_size: usize) {
    //util::init_logger(LogThread::Disabled); // Enable it for better debugging

    assert!(message_size <= transport.max_message_size());

    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let sent_message: Vec<u8> = (0..message_size).map(|_| rng.gen()).collect();

    let (node, listener) = node::split();

    let (_, receiver_addr) = node.network().listen(transport, LOCAL_ADDR).unwrap();
    let (receiver, _) = node.network().connect(transport, receiver_addr).unwrap();

    let mut _async_sender: Option<NamespacedThread<()>> = None;
    let mut received_message = Vec::new();

    listener.for_each(move |event| match event {
        NodeEvent::Waker(_) => panic!("{}", TIMEOUT_EVENT_RECV_ERR),
        NodeEvent::Network(net_event) => match net_event {
            NetEvent::Connected(endpoint, status) => {
                assert!(status);
                assert_eq!(receiver, endpoint);

                let node = node.clone();
                let sent_message = sent_message.clone();

                // Protocols as TCP blocks the sender if the receiver is not reading data
                // and its buffer is fill.
                _async_sender = Some(NamespacedThread::spawn("test-sender", move || {
                    let mut buffer = take_packet(sent_message.len());
                    buffer.append_slice(&sent_message.as_slice());
                    node.network().send(receiver, buffer);
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    assert!(node.network().remove(receiver.resource_id()));
                }));
            }
            NetEvent::Accepted(..) => (),
            NetEvent::Message(_, data) => {
                if transport.is_packet_based() {
                    received_message = data.peek().to_vec();
                    assert_eq!(sent_message, received_message);
                    node.stop();
                } else {
                    received_message.extend_from_slice(data.peek());
                }
            }
            NetEvent::Disconnected(_) => {
                assert_eq!(sent_message.len(), received_message.len());
                assert_eq!(sent_message, received_message);
                node.stop();
            }
        },
    });
}
