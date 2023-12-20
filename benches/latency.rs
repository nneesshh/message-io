use message_io::network::{Endpoint, Transport};
use message_io::node::{self, NodeHandler, NodeTask};
use message_io::util::thread::NamespacedThread;

use criterion::{criterion_group, criterion_main, Criterion};

use net_packet::take_small_packet;
#[cfg(feature = "websocket")]
use url::Url;

use std::io::{Read, Write};
#[cfg(feature = "udp")]
use std::net::UdpSocket;
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

lazy_static::lazy_static! {
    static ref TIMEOUT: Duration = Duration::from_millis(1);
}

fn init_connection(transport: Transport) -> (NodeTask, NodeHandler, Endpoint) {
    let (mux, handler) = node::split();

    let handler2 = handler.clone();
    let (promise, pinky) = pinky_swear::PinkySwear::<Endpoint>::new();
    let running = Arc::new(AtomicBool::new(true));
    let _thread = {
        let running = running.clone();
        NamespacedThread::spawn("perf-listening", move || {
            let server_addr = handler2.listen_sync(transport, "127.0.0.1:0").unwrap().1;
            let endpoint = handler2.connect_sync(transport, server_addr).unwrap().0;
            pinky.swear(endpoint);

            //
            running.store(false, Ordering::Relaxed);
        })
    };

    //
    let task = node::node_listener_for_each_async(mux, &handler, |_h, _| {
        //
    });

    // From here, the connection is performed independently of the transport used
    let endpoint = promise.wait();
    (task, handler, endpoint)
}

fn latency_by_native_tcp(c: &mut Criterion) {
    let msg = format!("latency by native Tcp");
    c.bench_function(&msg, |b| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let mut sender = TcpStream::connect(addr).unwrap();
        let (mut receiver, _) = listener.accept().unwrap();

        let mut buffer: [u8; 1] = [0; 1];

        b.iter(|| {
            sender.write(&[0xFF]).unwrap();
            receiver.read(&mut buffer).unwrap();
        });
    });
}

fn latency_by(c: &mut Criterion, transport: Transport) {
    let msg = format!("latency by {}", transport);
    let (_task, handler, endpoint) = init_connection(transport);

    c.bench_function(&msg, |b| {
        b.iter(|| {
            let mut buffer = take_small_packet();
            buffer.append_slice(&[0xFF]);
            handler.send_sync(endpoint, buffer);
            //handler.send(endpoint, buffer);
        });
    });
}

#[cfg(feature = "udp")]
fn latency_by_native_udp(c: &mut Criterion) {
    let msg = format!("latency by native Udp");
    c.bench_function(&msg, |b| {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = receiver.local_addr().unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.connect(addr).unwrap();

        let mut buffer: [u8; 1] = [0; 1];

        b.iter(|| {
            sender.send(&[0xFF]).unwrap();
            receiver.recv(&mut buffer).unwrap();
        });
    });
}

#[cfg(feature = "websocket")]
fn latency_by_native_web_socket(c: &mut Criterion) {
    let msg = format!("latency by native Ws");
    c.bench_function(&msg, |b| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let mut listen_thread = NamespacedThread::spawn("perf-listening", move || {
            ws_accept(listener.accept().unwrap().0).unwrap()
        });

        let url_addr = format!("ws://{}/socket", addr);
        let (mut sender, _) = ws_connect(Url::parse(&url_addr).unwrap()).unwrap();

        let mut receiver = listen_thread.join();

        let message = vec![0xFF];

        b.iter(|| {
            sender.write_message(Message::Binary(message.clone())).unwrap();
            receiver.read_message().unwrap();
        });
    });
}

fn latency(c: &mut Criterion) {
    #[cfg(feature = "udp")]
    latency_by_native_udp(c);
    #[cfg(feature = "tcp")]
    latency_by_native_tcp(c);
    #[cfg(feature = "websocket")]
    latency_by_native_web_socket(c);

    #[cfg(feature = "udp")]
    latency_by(c, Transport::Udp);
    #[cfg(feature = "tcp")]
    latency_by(c, Transport::Tcp);
    #[cfg(feature = "websocket")]
    latency_by(c, Transport::Ws);
}

criterion_group!(benches, latency);
criterion_main!(benches);
