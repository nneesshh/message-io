mod client;
mod common;
mod server;

use message_io::network::{ToRemoteAddr, Transport};

use std::net::ToSocketAddrs;

const HELP_MSG: &str = concat!(
    "Usage: ping-pong server <port>\n",
    "       pong-pong client (<ip>:<port> | url)"
);

pub fn main() {
    let args: Vec<String> = std::env::args().collect();

    let transport = Transport::Tcp;

    match args.get(1).unwrap_or(&"".into()).as_ref() {
        "client" => match args.get(2) {
            Some(remote_addr) => {
                let remote_addr = remote_addr.to_remote_addr().unwrap();
                client::run(transport, remote_addr);
            }
            None => return println!("{}", HELP_MSG),
        },
        "server" => {
            match args.get(2).unwrap_or(&"".into()).parse() {
                Ok(port) => {
                    let addr = ("0.0.0.0", port).to_socket_addrs().unwrap().next().unwrap();
                    server::run(transport, addr);
                }
                Err(_) => return println!("{}", HELP_MSG),
            };
        }
        _ => return println!("{}", HELP_MSG),
    }
}
