mod template;

#[cfg(feature = "tcp")]
pub mod tcp;
/*#[cfg(feature = "udp")]
pub mod udp;
*/
#[cfg(feature = "ssl")]
pub mod ssl;
/*#[cfg(feature = "websocket")]
pub mod ws;
*/

// Add new adapters here
// ...
