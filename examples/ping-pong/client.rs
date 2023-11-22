use super::common::{FromClientMessage, FromServerMessage};

use message_io::WakerCommand;
use message_io::network::{NetEvent, RemoteAddr, Transport};
use message_io::node::{self, NodeEvent};
use net_packet::take_packet;

pub fn run(transport: Transport, remote_addr: RemoteAddr) {
    let (handler, listener) = node::split();

    let (server_id, local_addr) =
        handler.network().connect(transport, remote_addr.clone()).unwrap();

    listener.for_each(move |event| match event {
        NodeEvent::Network(net_event) => match net_event {
            NetEvent::Connected(_, established) => {
                if established {
                    println!("Connected to server at {} by {}", server_id.addr(), transport);
                    println!("Client identified by local port: {}", local_addr.port());
                    handler.commands().post(handler.waker(), WakerCommand::Greet("Ping".to_owned()));
                } else {
                    println!("Can not connect to server at {} by {}", remote_addr, transport)
                }
            }
            NetEvent::Accepted(_, _) => unreachable!(), // Only generated when a listener accepts
            NetEvent::Message(_, input_data) => {
                let message: FromServerMessage = bincode::deserialize(input_data.peek()).unwrap();
                match message {
                    FromServerMessage::Pong(count) => {
                        println!("Pong from server: {} times", count)
                    }
                    FromServerMessage::UnknownPong => println!("Pong from server"),
                }
            }
            NetEvent::Disconnected(_) => {
                println!("Server is disconnected");
                handler.stop();
            }
        },
        NodeEvent::Waker(command) => match command {
            WakerCommand::Greet(_greet) => {
                let message = FromClientMessage::Ping;
                let output_data = bincode::serialize(&message).unwrap();
                let mut buffer = take_packet(output_data.len());
                buffer.append_slice(output_data.as_slice());
                handler.network().send(server_id, buffer);
                handler.commands().post(handler.waker(), WakerCommand::Greet("Pong".to_owned()));
            }
            _ => { std::unreachable!()}
        },
    });
}
