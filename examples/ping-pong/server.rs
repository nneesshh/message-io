use super::common::{FromClientMessage, FromServerMessage};

use message_io::network::{Endpoint, NetEvent, Transport};
use message_io::node::{self};

use hashbrown::HashMap;
use net_packet::take_packet;
use std::net::SocketAddr;

struct ClientInfo {
    count: usize,
}

pub fn run(transport: Transport, addr: SocketAddr) {
    let (engine, handler) = node::split();

    let mut clients: HashMap<Endpoint, ClientInfo> = HashMap::new();

    handler.network().listen(transport, addr, move |ret| {
        //
        match ret {
            Ok((_id, real_addr)) => println!("Server running at {} by {}", real_addr, transport),
            Err(_) => return println!("Can not listening at {} by {}", addr, transport),
        }
    });

    let handler2 = handler.clone();
    let mut task = node::node_listener_for_each_async(engine, &handler, move |event| match event.network() {
        NetEvent::Connected(_, _) => (), // Only generated at connect() calls.
        NetEvent::Accepted(endpoint, _listener_id) => {
            // Only connection oriented protocols will generate this event
            clients.insert(endpoint, ClientInfo { count: 0 });
            println!("Client ({}) connected (total clients: {})", endpoint.addr(), clients.len());
        }
        NetEvent::Message(endpoint, input_data) => {
            let message: FromClientMessage = bincode::deserialize(input_data.peek()).unwrap();
            match message {
                FromClientMessage::Ping => {
                    let message = match clients.get_mut(&endpoint) {
                        Some(client) => {
                            // For connection oriented protocols
                            client.count += 1;
                            println!("Ping from {}, {} times", endpoint.addr(), client.count);
                            FromServerMessage::Pong(client.count)
                        }
                        None => {
                            // For non-connection oriented protocols
                            println!("Ping from {}", endpoint.addr());
                            FromServerMessage::UnknownPong
                        }
                    };
                    let output_data = bincode::serialize(&message).unwrap();
                    let mut buffer = take_packet(output_data.len());
            buffer.append_slice(&output_data.as_slice());
            handler2.network().send(endpoint, buffer);
                }
            }
        }
        NetEvent::Disconnected(endpoint) => {
            // Only connection oriented protocols will generate this event
            clients.remove(&endpoint).unwrap();
            println!(
                "Client ({}) disconnected (total clients: {})",
                endpoint.addr(),
                clients.len()
            );
        }
    });
    task.wait();
}
