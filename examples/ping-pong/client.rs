use super::common::{FromClientMessage, FromServerMessage};

use message_io::WakerCommand;
use message_io::network::{NetEvent, RemoteAddr, Transport, Endpoint};
use message_io::node::{self, NodeEvent};
use net_packet::take_packet;

pub fn run(transport: Transport, remote_addr: RemoteAddr) {
    let (engine, handler) = node::split();
    let handler2 = handler.clone();
    let remote_addr2 = remote_addr.clone();

    let mut receiver_opt: Option<Endpoint> = None;
    let mut task = node::node_listener_for_each_async(engine, &handler, move |event| match event {
        NodeEvent::Network(net_event) => match net_event {
            NetEvent::Connected(endpoint, established) => {
                if established {
                    receiver_opt = Some(endpoint);
                    handler2.commands().post(handler2.waker(), WakerCommand::Greet("Ping".to_owned()));
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
                handler2.stop();
            }
        },
        NodeEvent::Waker(command) => match command {
            WakerCommand::Greet(_greet) => {

                let message = FromClientMessage::Ping;
                let output_data = bincode::serialize(&message).unwrap();
                let mut buffer = take_packet(output_data.len());
                buffer.append_slice(output_data.as_slice());

                let receiver = receiver_opt.as_ref().unwrap();
                handler2.network().send(receiver.clone(), buffer);
                std::thread::sleep(std::time::Duration::from_secs(1));
                handler2.commands().post(handler2.waker(), WakerCommand::Greet("Ping".to_owned()));
            }
            _ => { std::unreachable!()}
        },
    });

    let (server_id, local_addr) =
    handler.network().connect_sync(transport, remote_addr2).unwrap();

    println!("Connected to server at {} by {}", server_id.addr(), transport);
    println!("Client identified by local port: {}", local_addr.port());
    task.wait();

}
