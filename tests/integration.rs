use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use message_io::network::{NetEvent, Transport};
use message_io::node::{self, NodeHandler};
use message_io::node_event::NodeEvent;
use message_io::util::thread::NamespacedThread;

use net_packet::take_packet;
use parking_lot::Mutex;
use test_case::test_case;

use rand::{Rng, SeedableRng};

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
    ///
    #[allow(dead_code)]
    pub fn init_logger() {
        let log_path = std::path::PathBuf::from("log");
        let log_level = my_logger::LogLevel::Info as u16;
        my_logger::init(&log_path, "message-io", log_level, false);
    }
}

fn start_echo_server(
    transport: Transport,
    expected_clients: usize,
) -> (NamespacedThread<()>, SocketAddr) {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let thread = NamespacedThread::spawn("test-server", move || {
        let messages_received = AtomicUsize::new(0);
        let disconnections = AtomicUsize::new(0);
        let clients = Mutex::new(HashSet::new());

        let (mux, handler) = node::split();

        handler.listen(
            transport,
            LOCAL_ADDR,
            Box::new(move |_h: &NodeHandler, ret| {
                //
                if let Ok((_listenr_id, server_addr)) = ret {
                    tx.send(server_addr).unwrap();
                }
            }),
        );

        let listener_id_opt = Mutex::new(None);
        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, e| {
            match e {
                NodeEvent::Waker(_) => {
                    //
                    log::trace!("waker");
                }
                NodeEvent::Network(net_event) => match net_event {
                    NetEvent::Connected(..) => unreachable!(),
                    NetEvent::Accepted(endpoint, id) => {
                        {
                            let mut guard = listener_id_opt.lock();
                            *guard = Some(id);
                        }
                        match transport.is_connection_oriented() {
                            true => {
                                //
                                let mut guard = clients.lock();
                                assert!(guard.insert(endpoint));
                            }
                            false => unreachable!(),
                        }
                    }
                    NetEvent::Message(endpoint, data) => {
                        assert_eq!(MIN_MESSAGE, data.peek());

                        h.send(endpoint, data);

                        messages_received.fetch_add(1, Ordering::Relaxed);
                        log::info!(
                            "messages_received={}",
                            messages_received.load(Ordering::Relaxed)
                        );

                        if !transport.is_connection_oriented() {
                            // We assume here that if the protocol is not
                            // connection-oriented it will no create a resource.
                            // The remote will be managed from the listener resource
                            {
                                let guard = listener_id_opt.lock();
                                let listener_id = guard.unwrap();
                                assert_eq!(listener_id, endpoint.resource_id());
                            }
                            if messages_received.load(Ordering::Relaxed) == expected_clients {
                                h.stop() //Exit from thread.
                            }
                        }
                    }
                    NetEvent::Disconnected(endpoint) => {
                        match transport.is_connection_oriented() {
                            true => {
                                disconnections.fetch_add(1, Ordering::Relaxed);

                                //
                                {
                                    let mut guard = clients.lock();
                                    assert!(guard.remove(&endpoint));
                                }
                                if disconnections.load(Ordering::Relaxed) == expected_clients {
                                    assert_eq!(
                                        expected_clients,
                                        messages_received.load(Ordering::Relaxed)
                                    );
                                    {
                                        let guard = clients.lock();
                                        let clients_len = guard.len();
                                        assert_eq!(0, clients_len);
                                    }
                                    h.stop() //Exit from thread.
                                }
                            }
                            false => unreachable!(),
                        }
                    }
                },
            }
        });
        task.wait();
    });

    let server_addr = rx.recv_timeout(*TIMEOUT).expect(TIMEOUT_EVENT_RECV_ERR);
    (thread, server_addr)
}

fn start_echo_client_manager(transport: Transport, server_addr: SocketAddr, clients_number: usize) {
    let mut thread = NamespacedThread::spawn("test-client", move || {
        let (mux, handler) = node::split();

        let clients = Mutex::new(HashSet::new());
        let received = AtomicUsize::new(0);

        for _ in 0..clients_number {
            handler.connect(transport, server_addr, Box::new(|_h: &NodeHandler, _| {}));
        }

        let mut task = node::node_listener_for_each_async(
            mux,
            &handler,
            Box::new(move |h: &NodeHandler, e| {
                //
                match e {
                    NodeEvent::Waker(_) => panic!("{}", TIMEOUT_EVENT_RECV_ERR),
                    NodeEvent::Network(net_event) => match net_event {
                        NetEvent::Connected(server, status) => {
                            assert!(status);
                            let mut buffer = take_packet(MIN_MESSAGE.len());
                            buffer.append_slice(MIN_MESSAGE);
                            h.send(server, buffer);

                            //
                            {
                                let mut guard = clients.lock();
                                assert!(guard.insert(server));
                            }
                        }
                        NetEvent::Message(endpoint, data) => {
                            //
                            {
                                let mut guard = clients.lock();
                                assert!(guard.remove(&endpoint));
                            }

                            assert_eq!(MIN_MESSAGE, data.peek());
                            h.close(endpoint.resource_id());

                            received.fetch_add(1, Ordering::Relaxed);
                            if received.load(Ordering::Relaxed) == clients_number {
                                h.stop(); //Exit from thread.
                            }
                        }
                        NetEvent::Accepted(..) => unreachable!(),
                        NetEvent::Disconnected(_) => unreachable!(),
                    },
                }
            }),
        );
        task.wait();
    });

    thread.join();
}

#[cfg_attr(feature = "tcp", test_case(Transport::Tcp, 100))]
// NOTE: A medium-high `clients` value can exceeds the "open file" limits of an OS in CI
// with an obfuscated error message.
fn echo(transport: Transport, clients: usize) {
    util::init_logger(); // Enable it for better debugging

    let (mut server_thread, server_addr) = start_echo_server(transport, clients);
    start_echo_client_manager(transport, server_addr, clients);
    server_thread.join();
}

#[cfg_attr(feature = "tcp", test_case(Transport::Tcp, BIG_MESSAGE_SIZE))]
fn message_size(transport: Transport, message_size: usize) {
    //util::init_logger(); // Enable it for better debugging

    assert!(message_size <= transport.max_message_size());

    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let sent_message: Vec<u8> = (0..message_size).map(|_| rng.gen()).collect();

    let (mux, handler) = node::split();

    handler.listen(transport, LOCAL_ADDR, move |h, ret| {
        //
        if let Ok((_, server_addr)) = ret {
            h.connect(transport, server_addr, Box::new(|_h: &NodeHandler, _| {}));
        }
    });

    let received_message = Mutex::new(Vec::new());

    let mut task = node::node_listener_for_each_async(mux, &handler, move |h, event| match event {
        NodeEvent::Waker(_) => panic!("{}", TIMEOUT_EVENT_RECV_ERR),
        NodeEvent::Network(net_event) => match net_event {
            NetEvent::Connected(receiver, status) => {
                assert!(status);

                let sent_message = sent_message.clone();

                // Protocols as TCP blocks the sender if the receiver is not reading data
                // and its buffer is fill.
                let handler2 = h.clone();
                let _ = Some(NamespacedThread::spawn("test-sender", move || {
                    let mut buffer = take_packet(sent_message.len());
                    buffer.append_slice(&sent_message.as_slice());
                    handler2.send(receiver, buffer);
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    handler2.close(receiver.resource_id());
                }));
            }
            NetEvent::Accepted(..) => (),
            NetEvent::Message(_, buffer) => {
                let data = buffer.peek();
                log::info!("receive data len={}", data.len());
                let mut guard = received_message.lock();
                if transport.is_packet_based() {
                    *guard = data.to_vec();
                    assert_eq!(sent_message, *guard);
                    h.stop();
                } else {
                    guard.extend_from_slice(data);
                }
            }
            NetEvent::Disconnected(_) => {
                let guard = received_message.lock();
                assert_eq!(sent_message.len(), guard.len());
                assert_eq!(sent_message, *guard);
                h.stop();
            }
        },
    });
    task.wait();
}
