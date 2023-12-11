///
pub mod tcp_driver;

///
pub mod tcp_adapter;

///
pub mod tcp_remote;

///
pub use tcp_adapter::{check_tcp_stream_ready, tcp_stream_to_socket};
